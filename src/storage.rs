use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, Transaction, params};

use crate::cli::{IndexArgs, QueryArgs};
use crate::extract::extract_facts;
use crate::model::{Confidence, Fact, FactRole};
use crate::source_link::store_plugin_mirror_url;
use crate::walker::{CandidateFile, PluginInfo, WalkStats, discover_candidates};

const SCHEMA_VERSION: i64 = 1;

pub fn build_index(args: IndexArgs, verbose: bool) -> Result<()> {
    let started_at = Instant::now();
    prepare_output_location(&args.out, args.force)?;

    let (candidates, walk_stats) =
        discover_candidates(&args).context("discover plugin candidate files")?;

    let include_snippets = !args.no_snippets;
    let extracted_candidates = extract_candidates(candidates, include_snippets, &args.threads)?;
    let mut connection = open_fresh_database(&args.out, args.force)?;
    configure_bulk_write_database(&connection).context("configure SQLite bulk write settings")?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .context("enable SQLite foreign keys")?;

    let summary = {
        let transaction = connection
            .transaction()
            .context("start SQLite index transaction")?;

        create_schema(&transaction).context("create SQLite index schema")?;
        let summary = insert_index_data(
            &transaction,
            &args,
            extracted_candidates,
            walk_stats,
            include_snippets,
        )
        .context("insert extracted facts")?;
        create_indexes(&transaction).context("create SQLite index indexes")?;
        insert_metadata(&transaction, &args, &summary, include_snippets)
            .context("insert SQLite index metadata")?;

        transaction
            .commit()
            .context("commit SQLite index transaction")?;
        summary
    };

    if verbose {
        print_index_summary(&args.out, &summary);
    }

    print_index_duration(&args.out, &summary, started_at.elapsed());

    Ok(())
}

pub fn query_index(args: QueryArgs) -> Result<()> {
    let connection = Connection::open_with_flags(&args.index, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open SQLite index {}", path_display(&args.index)))?;

    let result = query_surface(
        &connection,
        &args.surface,
        args.include_low_confidence,
        args.only_high_confidence,
        args.max_evidence,
    )
    .with_context(|| format!("query surface {}", args.surface))?;

    print_query_result(&result);
    Ok(())
}

#[derive(Debug, Default)]
struct BuildSummary {
    walk_plugins: usize,
    files_seen: usize,
    candidates: usize,
    skipped_large: usize,
    skipped_unsupported: usize,
    plugin_rows: usize,
    surface_rows: usize,
    impact_rows: usize,
    facts_extracted: usize,
    facts_stored: usize,
    evidence_rows: usize,
}

#[derive(Debug)]
struct ImpactAggregate {
    usage_count: i64,
    confidence: Confidence,
}

#[derive(Debug)]
struct ExtractedCandidate {
    plugin: PluginInfo,
    relative_path: PathBuf,
    facts: Vec<Fact>,
}

#[derive(Debug, PartialEq, Eq)]
struct QueryResult {
    surface: String,
    affected_plugins: usize,
    total_usages: usize,
    evidence: Vec<QueryEvidence>,
}

#[derive(Debug, PartialEq, Eq)]
struct QueryEvidence {
    plugin_name: String,
    plugin_folder: String,
    file_path: String,
    line: usize,
    column: Option<usize>,
    usage_kind: String,
    snippet: Option<String>,
    confidence: Confidence,
}

fn prepare_output_location(out: &Path, force: bool) -> Result<()> {
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("create index output directory {}", path_display(parent)))?;
    }

    if out.is_dir() {
        bail!("index output path is a directory: {}", path_display(out));
    }

    if out.exists() && !force {
        bail!(
            "index output already exists: {} (use --force to overwrite)",
            path_display(out)
        );
    }

    Ok(())
}

fn open_fresh_database(out: &Path, force: bool) -> Result<Connection> {
    if out.exists() {
        if !force {
            bail!(
                "index output already exists: {} (use --force to overwrite)",
                path_display(out)
            );
        }

        if out.is_dir() {
            bail!("index output path is a directory: {}", path_display(out));
        }

        fs::remove_file(out)
            .with_context(|| format!("remove existing SQLite index {}", path_display(out)))?;
    }

    Connection::open(out).with_context(|| format!("create SQLite index {}", path_display(out)))
}

fn configure_bulk_write_database(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "
        pragma journal_mode = off;
        pragma synchronous = off;
        pragma temp_store = memory;
        pragma locking_mode = exclusive;
        pragma cache_size = -200000;
        ",
    )?;

    Ok(())
}

fn create_schema(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        "
        pragma user_version = 1;

        create table metadata (
            key text primary key,
            value text not null
        );

        create table plugin (
            id integer primary key,
            name text not null,
            version text,
            path text not null
        );

        create table file (
            id integer primary key,
            plugin_id integer not null,
            path text not null,
            hash blob,
            foreign key (plugin_id) references plugin(id)
        );

        create table surface (
            id integer primary key,
            key text not null unique,
            kind text not null
        );

        create table impact (
            surface_id integer not null,
            plugin_id integer not null,
            usage_count integer not null,
            confidence integer not null,
            primary key (surface_id, plugin_id),
            foreign key (surface_id) references surface(id),
            foreign key (plugin_id) references plugin(id)
        );

        create table evidence (
            surface_id integer not null,
            plugin_id integer not null,
            file_id integer not null,
            line integer not null,
            column integer,
            usage_kind text not null,
            snippet text,
            confidence integer not null,
            foreign key (surface_id) references surface(id),
            foreign key (plugin_id) references plugin(id),
            foreign key (file_id) references file(id)
        );
        ",
    )?;

    Ok(())
}

fn create_indexes(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        "
        create index evidence_surface_plugin on evidence(surface_id, plugin_id);
        create index evidence_confidence on evidence(confidence);
        create index impact_surface on impact(surface_id);
        create index file_plugin on file(plugin_id);
        ",
    )?;

    Ok(())
}

fn extract_candidates(
    candidates: Vec<CandidateFile>,
    include_snippets: bool,
    threads: &str,
) -> Result<Vec<ExtractedCandidate>> {
    match parse_threads(threads)? {
        Some(threads) => rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .context("build extraction thread pool")?
            .install(|| extract_candidates_parallel(candidates, include_snippets)),
        None => extract_candidates_parallel(candidates, include_snippets),
    }
}

fn extract_candidates_parallel(
    candidates: Vec<CandidateFile>,
    include_snippets: bool,
) -> Result<Vec<ExtractedCandidate>> {
    candidates
        .into_par_iter()
        .map(|candidate| extract_candidate(candidate, include_snippets))
        .collect()
}

fn extract_candidate(
    candidate: CandidateFile,
    include_snippets: bool,
) -> Result<ExtractedCandidate> {
    let content = read_candidate_content(&candidate.absolute_path)?;
    let facts = extract_facts(
        candidate.language,
        &content,
        &candidate.relative_path,
        FactRole::Usage,
        include_snippets,
    )
    .with_context(|| {
        format!(
            "extract facts from candidate file {}",
            path_display(&candidate.absolute_path)
        )
    })?;

    Ok(ExtractedCandidate {
        plugin: candidate.plugin,
        relative_path: candidate.relative_path,
        facts,
    })
}

fn parse_threads(value: &str) -> Result<Option<usize>> {
    if value.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }

    let threads = value
        .parse::<usize>()
        .with_context(|| format!("invalid thread count: {value}"))?;

    if threads == 0 {
        bail!("thread count must be greater than zero");
    }

    Ok(Some(threads))
}

fn insert_index_data(
    transaction: &Transaction<'_>,
    args: &IndexArgs,
    candidates: Vec<ExtractedCandidate>,
    walk_stats: WalkStats,
    include_snippets: bool,
) -> Result<BuildSummary> {
    let mut summary = BuildSummary {
        walk_plugins: walk_stats.plugins,
        files_seen: walk_stats.files_seen,
        candidates: walk_stats.candidates,
        skipped_large: walk_stats.skipped_large,
        skipped_unsupported: walk_stats.skipped_unsupported,
        ..BuildSummary::default()
    };

    let mut plugin_ids: HashMap<(String, Option<String>, String), i64> = HashMap::new();
    let mut surface_ids: HashMap<String, i64> = HashMap::new();
    let mut impacts: HashMap<(i64, i64), ImpactAggregate> = HashMap::new();

    {
        let mut insert_plugin = transaction.prepare(
            "
            insert into plugin (name, version, path)
            values (?1, ?2, ?3)
            ",
        )?;
        let mut insert_file = transaction.prepare(
            "
            insert into file (plugin_id, path, hash)
            values (?1, ?2, ?3)
            ",
        )?;
        let mut insert_surface = transaction.prepare(
            "
            insert into surface (key, kind)
            values (?1, ?2)
            ",
        )?;
        let mut insert_evidence = transaction.prepare(
            "
            insert into evidence (
                surface_id,
                plugin_id,
                file_id,
                line,
                column,
                usage_kind,
                snippet,
                confidence
            )
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
        )?;

        for candidate in candidates {
            let plugin_path = plugin_folder(&candidate.plugin.root);
            let plugin_key = (
                candidate.plugin.name.clone(),
                candidate.plugin.version.clone(),
                plugin_path,
            );
            let plugin_id = if let Some(plugin_id) = plugin_ids.get(&plugin_key) {
                *plugin_id
            } else {
                insert_plugin.execute(params![
                    &plugin_key.0,
                    plugin_key.1.as_deref(),
                    &plugin_key.2,
                ])?;
                let plugin_id = transaction.last_insert_rowid();
                plugin_ids.insert(plugin_key, plugin_id);
                plugin_id
            };

            insert_file.execute(params![
                plugin_id,
                path_to_string(&candidate.relative_path),
                Option::<Vec<u8>>::None,
            ])?;
            let file_id = transaction.last_insert_rowid();

            summary.facts_extracted += candidate.facts.len();

            for fact in candidate.facts {
                if fact.role != FactRole::Usage
                    || !fact.confidence.include(args.include_low_confidence, false)
                {
                    continue;
                }

                let surface_key = fact.surface.as_str().to_owned();
                let surface_id = if let Some(surface_id) = surface_ids.get(&surface_key) {
                    *surface_id
                } else {
                    insert_surface.execute(params![&surface_key, fact.kind.as_str()])?;
                    let surface_id = transaction.last_insert_rowid();
                    surface_ids.insert(surface_key, surface_id);
                    surface_id
                };

                let confidence = fact.confidence;
                let snippet = if include_snippets {
                    fact.evidence.snippet
                } else {
                    None
                };

                insert_evidence.execute(params![
                    surface_id,
                    plugin_id,
                    file_id,
                    fact.evidence.line as i64,
                    fact.evidence.column.map(|column| column as i64),
                    fact.usage_kind,
                    snippet,
                    confidence_value(confidence),
                ])?;

                let impact = impacts
                    .entry((surface_id, plugin_id))
                    .or_insert(ImpactAggregate {
                        usage_count: 0,
                        confidence,
                    });
                impact.usage_count += 1;
                impact.confidence = impact.confidence.max(confidence);

                summary.facts_stored += 1;
                summary.evidence_rows += 1;
            }
        }
    }

    summary.plugin_rows = plugin_ids.len();
    summary.surface_rows = surface_ids.len();
    summary.impact_rows = impacts.len();

    let mut insert_impact = transaction.prepare(
        "
        insert into impact (surface_id, plugin_id, usage_count, confidence)
        values (?1, ?2, ?3, ?4)
        ",
    )?;

    for ((surface_id, plugin_id), impact) in impacts {
        insert_impact.execute(params![
            surface_id,
            plugin_id,
            impact.usage_count,
            confidence_value(impact.confidence),
        ])?;
    }

    Ok(summary)
}

fn insert_metadata(
    transaction: &Transaction<'_>,
    args: &IndexArgs,
    summary: &BuildSummary,
    include_snippets: bool,
) -> Result<()> {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    let metadata = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("created_at_unix", created_at),
        ("corpus_root", path_file_name(&args.plugins)),
        (
            "include_low_confidence",
            args.include_low_confidence.to_string(),
        ),
        ("snippets_enabled", include_snippets.to_string()),
        ("walk_plugins", summary.walk_plugins.to_string()),
        ("files_seen", summary.files_seen.to_string()),
        ("candidates", summary.candidates.to_string()),
        ("skipped_large", summary.skipped_large.to_string()),
        (
            "skipped_unsupported",
            summary.skipped_unsupported.to_string(),
        ),
        ("facts_extracted", summary.facts_extracted.to_string()),
        ("facts_stored", summary.facts_stored.to_string()),
        ("plugin_rows", summary.plugin_rows.to_string()),
        ("surface_rows", summary.surface_rows.to_string()),
        ("impact_rows", summary.impact_rows.to_string()),
        ("evidence_rows", summary.evidence_rows.to_string()),
    ];

    let mut insert_metadata =
        transaction.prepare("insert into metadata (key, value) values (?1, ?2)")?;

    for (key, value) in metadata {
        insert_metadata.execute(params![key, value])?;
    }

    Ok(())
}

fn query_surface(
    connection: &Connection,
    surface: &str,
    include_low_confidence: bool,
    only_high_confidence: bool,
    max_evidence: usize,
) -> Result<QueryResult> {
    let min_confidence = if include_low_confidence { 1 } else { 2 };
    let require_high = if only_high_confidence { 1 } else { 0 };

    let (affected_plugins, total_usages): (i64, i64) = connection.query_row(
        "
        select count(distinct e.plugin_id), count(*)
        from evidence e
        join surface s on s.id = e.surface_id
        where s.key = ?1
            and e.confidence >= ?2
            and (?3 = 0 or e.confidence = 3)
        ",
        params![surface, min_confidence, require_high],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let mut statement = connection.prepare(
        "
        select
            p.name,
            p.path,
            f.path,
            e.line,
            e.column,
            e.usage_kind,
            e.snippet,
            e.confidence
        from evidence e
        join surface s on s.id = e.surface_id
        join plugin p on p.id = e.plugin_id
        join file f on f.id = e.file_id
        where s.key = ?1
            and e.confidence >= ?2
            and (?3 = 0 or e.confidence = 3)
        order by e.confidence desc, p.name asc, f.path asc, e.line asc, coalesce(e.column, 0) asc, e.usage_kind asc
        limit ?4
        ",
    )?;
    let max_evidence = i64::try_from(max_evidence).unwrap_or(i64::MAX);
    let evidence = statement
        .query_map(
            params![surface, min_confidence, require_high, max_evidence],
            |row| {
                let confidence: i64 = row.get(7)?;
                Ok(QueryEvidence {
                    plugin_name: row.get(0)?,
                    plugin_folder: row.get(1)?,
                    file_path: row.get(2)?,
                    line: i64_to_usize(row.get(3)?),
                    column: row.get::<_, Option<i64>>(4)?.map(i64_to_usize),
                    usage_kind: row.get(5)?,
                    snippet: row.get(6)?,
                    confidence: Confidence::from_i64(confidence),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(QueryResult {
        surface: surface.to_owned(),
        affected_plugins: i64_to_usize(affected_plugins),
        total_usages: i64_to_usize(total_usages),
        evidence,
    })
}

fn print_index_summary(out: &Path, summary: &BuildSummary) {
    eprintln!("Index: {}", path_display(out));
    eprintln!(
        "Discovered {} candidate files from {} plugins ({} files seen, {} large skipped, {} unsupported skipped).",
        summary.candidates,
        summary.walk_plugins,
        summary.files_seen,
        summary.skipped_large,
        summary.skipped_unsupported,
    );
    eprintln!(
        "Stored {} evidence rows from {} extracted facts across {} surfaces, {} plugins, and {} plugin impacts.",
        summary.evidence_rows,
        summary.facts_extracted,
        summary.surface_rows,
        summary.plugin_rows,
        summary.impact_rows,
    );
}

fn print_index_duration(out: &Path, summary: &BuildSummary, elapsed: Duration) {
    eprintln!(
        "Indexed {} plugins, {} candidate files, and {} evidence rows into {} in {}.",
        summary.plugin_rows,
        summary.candidates,
        summary.evidence_rows,
        path_display(out),
        format_duration(elapsed),
    );
}

fn print_query_result(result: &QueryResult) {
    println!("Surface: {}", result.surface);
    println!("Affected plugins: {}", result.affected_plugins);
    println!("Usages: {}", result.total_usages);

    if result.evidence.is_empty() && result.total_usages == 0 {
        println!("No evidence found.");
        return;
    }

    if result.evidence.is_empty() {
        println!("Evidence output limited to 0 rows.");
        return;
    }

    println!();

    if result.evidence.len() < result.total_usages {
        println!(
            "Showing {} of {} evidence rows.",
            result.evidence.len(),
            result.total_usages,
        );
        println!();
    }

    let plugin_width = result
        .evidence
        .iter()
        .map(|evidence| evidence.plugin_name.len())
        .max()
        .unwrap_or("Plugin".len());
    let location_width = result
        .evidence
        .iter()
        .map(|evidence| evidence.location().len())
        .max()
        .unwrap_or("Location".len());
    let usage_width = result
        .evidence
        .iter()
        .map(|evidence| evidence.usage_kind.len())
        .max()
        .unwrap_or("Usage".len());

    for evidence in &result.evidence {
        println!(
            "{:<plugin_width$}  {:<location_width$}  {:<usage_width$}  {}",
            evidence.plugin_name,
            evidence.location(),
            evidence.usage_kind,
            evidence.confidence.as_str(),
        );

        if let Some(snippet) = evidence
            .snippet
            .as_deref()
            .map(str::trim)
            .filter(|snippet| !snippet.is_empty())
        {
            println!("{}{}", " ".repeat(plugin_width + 2), snippet);
        }

        println!(
            "{}GitHub: {}",
            " ".repeat(plugin_width + 2),
            evidence.github_url()
        );
    }
}

impl QueryEvidence {
    fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.line)
    }

    fn github_url(&self) -> String {
        store_plugin_mirror_url(&self.plugin_folder, Path::new(&self.file_path), self.line)
    }
}

fn confidence_value(confidence: Confidence) -> i64 {
    match confidence {
        Confidence::Low => 1,
        Confidence::Medium => 2,
        Confidence::High => 3,
    }
}

fn i64_to_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn path_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path_display(path))
}

fn path_display(path: &Path) -> String {
    path.display().to_string()
}

fn plugin_folder(path: &Path) -> String {
    path_file_name(path)
}

fn format_duration(duration: Duration) -> String {
    let total_millis = duration.as_millis();
    let minutes = total_millis / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    if minutes > 0 {
        format!("{minutes}m {seconds}.{millis:03}s")
    } else {
        format!("{seconds}.{millis:03}s")
    }
}

fn read_candidate_content(path: &Path) -> Result<String> {
    let bytes =
        fs::read(path).with_context(|| format!("read candidate file {}", path_display(path)))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_output_location_refuses_existing_file_without_force() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let index_path = tempdir.path().join("index.sqlite");
        fs::write(&index_path, "existing").expect("write existing file");

        let error = prepare_output_location(&index_path, false).expect_err("should refuse");
        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read_to_string(&index_path).unwrap(), "existing");
    }

    #[test]
    fn open_fresh_database_creates_parent_dirs_and_overwrites_when_forced() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let index_path = tempdir.path().join("nested").join("index.sqlite");

        prepare_output_location(&index_path, true).expect("prepare output");
        let connection = open_fresh_database(&index_path, true).expect("open database");
        connection
            .execute("create table marker (value text)", [])
            .expect("create marker table");
        drop(connection);

        assert!(index_path.exists());
    }

    #[test]
    fn query_surface_filters_confidence_and_limits_evidence() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let index_path = tempdir.path().join("index.sqlite");
        let mut connection = Connection::open(index_path).expect("open database");
        seed_query_fixture(&mut connection);

        let default = query_surface(
            &connection,
            "service:id:shopware.foo",
            false,
            false,
            usize::MAX,
        )
        .expect("query default confidence");
        assert_eq!(default.affected_plugins, 1);
        assert_eq!(default.total_usages, 2);
        assert_eq!(default.evidence.len(), 2);
        assert!(
            default
                .evidence
                .iter()
                .all(|evidence| evidence.confidence != Confidence::Low)
        );

        let include_low = query_surface(&connection, "service:id:shopware.foo", true, false, 1)
            .expect("query with low confidence");
        assert_eq!(include_low.affected_plugins, 2);
        assert_eq!(include_low.total_usages, 4);
        assert_eq!(include_low.evidence.len(), 1);

        let only_high = query_surface(
            &connection,
            "service:id:shopware.foo",
            true,
            true,
            usize::MAX,
        )
        .expect("query only high confidence");
        assert_eq!(only_high.affected_plugins, 1);
        assert_eq!(only_high.total_usages, 1);
        assert_eq!(only_high.evidence[0].confidence, Confidence::High);
    }

    fn seed_query_fixture(connection: &mut Connection) {
        let transaction = connection.transaction().expect("start transaction");
        create_schema(&transaction).expect("create schema");

        transaction
            .execute(
                "insert into plugin (id, name, version, path) values (1, 'PluginA', null, 'PluginA')",
                [],
            )
            .expect("insert plugin a");
        transaction
            .execute(
                "insert into plugin (id, name, version, path) values (2, 'PluginB', null, 'PluginB')",
                [],
            )
            .expect("insert plugin b");
        transaction
            .execute(
                "insert into file (id, plugin_id, path, hash) values (1, 1, 'src/A.php', null)",
                [],
            )
            .expect("insert file a");
        transaction
            .execute(
                "insert into file (id, plugin_id, path, hash) values (2, 2, 'src/B.php', null)",
                [],
            )
            .expect("insert file b");
        transaction
            .execute(
                "insert into surface (id, key, kind) values (1, 'service:id:shopware.foo', 'service-id')",
                [],
            )
            .expect("insert surface");

        for (plugin_id, file_id, line, usage_kind, confidence) in [
            (1, 1, 10, "constructor", Confidence::High),
            (1, 1, 20, "string-ref", Confidence::Medium),
            (1, 1, 30, "comment", Confidence::Low),
            (2, 2, 40, "comment", Confidence::Low),
        ] {
            transaction
                .execute(
                    "
                    insert into evidence (
                        surface_id,
                        plugin_id,
                        file_id,
                        line,
                        column,
                        usage_kind,
                        snippet,
                        confidence
                    )
                    values (1, ?1, ?2, ?3, null, ?4, null, ?5)
                    ",
                    params![
                        plugin_id,
                        file_id,
                        line,
                        usage_kind,
                        confidence_value(confidence)
                    ],
                )
                .expect("insert evidence");
        }

        transaction
            .execute(
                "
                insert into impact (surface_id, plugin_id, usage_count, confidence)
                values (1, 1, 3, 3), (1, 2, 1, 1)
                ",
                [],
            )
            .expect("insert impact");

        transaction.commit().expect("commit fixture");
    }
}
