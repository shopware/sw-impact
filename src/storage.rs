use std::collections::HashMap;
use std::env;
use std::fmt::Write;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags, Transaction, params};

use crate::cli::{IndexArgs, QueryArgs};
use crate::extract::extract_facts;
use crate::model::{Confidence, Fact, FactRole};
use crate::report::{
    EvidenceDisplay, ReportOptions, SEPARATOR, affected_plugins_summary, write_aligned_key_values,
    write_dashed_block, write_numbered_evidence_entries,
};
use crate::source_link::store_plugin_mirror_url;
use crate::walker::{CandidateFile, Language, PluginInfo, WalkStats, discover_candidates};

const SCHEMA_VERSION: i64 = 1;
const EXTRACTION_CHUNK_SIZE: usize = 256;

pub fn build_index(args: IndexArgs, verbose: bool) -> Result<()> {
    let started_at = Instant::now();
    prepare_output_location(&args.out, args.force)?;

    let discover_started_at = Instant::now();
    let (candidates, walk_stats) =
        discover_candidates(&args).context("discover plugin candidate files")?;
    let discover_elapsed = discover_started_at.elapsed();

    let sqlite_started_at = Instant::now();
    let mut connection = open_fresh_database(&args.out, args.force)?;
    configure_bulk_write_database(&connection).context("configure SQLite bulk write settings")?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .context("enable SQLite foreign keys")?;

    let insert_result = {
        let transaction = connection
            .transaction()
            .context("start SQLite index transaction")?;

        create_schema(&transaction).context("create SQLite index schema")?;
        let include_snippets = !args.no_snippets;
        let insert_result = insert_index_data(
            &transaction,
            &args,
            candidates,
            walk_stats,
            include_snippets,
        )
        .context("insert extracted facts")?;
        create_indexes(&transaction).context("create SQLite index indexes")?;
        insert_metadata(
            &transaction,
            &args,
            &insert_result.summary,
            include_snippets,
        )
        .context("insert SQLite index metadata")?;

        transaction
            .commit()
            .context("commit SQLite index transaction")?;
        insert_result
    };
    let sqlite_elapsed = sqlite_started_at
        .elapsed()
        .saturating_sub(insert_result.extract_elapsed);

    if verbose {
        print_index_summary(&args.out, &insert_result.summary);
        print_index_timings(
            discover_elapsed,
            insert_result.extract_elapsed,
            sqlite_elapsed,
            &insert_result.extraction_timings,
        );
    }

    print_index_duration(&args.out, &insert_result.summary, started_at.elapsed());

    Ok(())
}

pub fn query_index(args: QueryArgs) -> Result<()> {
    let started_at = Instant::now();
    let connection = Connection::open_with_flags(&args.index, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open SQLite index {}", path_display(&args.index)))?;

    let mut result = query_surface(
        &connection,
        &args.pattern,
        args.include_low_confidence,
        args.only_high_confidence,
        args.max_evidence,
    )
    .with_context(|| format!("query surface pattern {}", args.pattern))?;
    result.elapsed = Some(started_at.elapsed());

    print_query_result(
        &result,
        ReportOptions {
            terminal_links: stdout_supports_links(),
        },
    );
    Ok(())
}

fn stdout_supports_links() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }

    if env::var_os("SW_IMPACT_NO_LINKS").is_some() {
        return false;
    }

    env::var("TERM").map_or(true, |term| term != "dumb")
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
struct IndexInsertResult {
    summary: BuildSummary,
    extraction_timings: ExtractionTimings,
    extract_elapsed: Duration,
}

#[derive(Debug)]
struct IndexInsertState {
    summary: BuildSummary,
    plugin_ids: HashMap<(String, Option<String>, String), i64>,
    surface_ids: HashMap<String, i64>,
}

#[derive(Debug)]
struct ExtractedCandidate {
    plugin: PluginInfo,
    relative_path: PathBuf,
    language: Language,
    read_duration: Duration,
    extract_duration: Duration,
    facts: Vec<Fact>,
}

#[derive(Debug, PartialEq, Eq)]
struct QueryResult {
    query: SurfaceQuery,
    matched_surfaces: usize,
    affected_plugins: usize,
    total_usages: usize,
    indexed_plugins: usize,
    elapsed: Option<Duration>,
    surfaces: Vec<QuerySurfaceResult>,
}

#[derive(Debug, PartialEq, Eq)]
struct QuerySurfaceResult {
    surface: String,
    kind: String,
    affected_plugins: usize,
    usage_count: usize,
    confidence: Confidence,
    evidence: Vec<QueryEvidence>,
}

#[derive(Debug, PartialEq, Eq)]
struct QueryEvidence {
    surface: String,
    plugin_name: String,
    plugin_version: Option<String>,
    plugin_folder: String,
    file_path: String,
    line: usize,
    column: Option<usize>,
    usage_kind: String,
    snippet: Option<String>,
    confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SurfaceQuery {
    Exact(String),
    Pattern { input: String, like_pattern: String },
}

#[derive(Debug, Default)]
struct ExtractionTimings {
    languages: HashMap<Language, LanguageExtractionTiming>,
}

#[derive(Debug, Default)]
struct LanguageExtractionTiming {
    files: usize,
    facts: usize,
    read_duration: Duration,
    extract_duration: Duration,
}

impl ExtractionTimings {
    fn add_candidates(&mut self, candidates: &[ExtractedCandidate]) {
        for candidate in candidates {
            let timing = self.languages.entry(candidate.language).or_default();
            timing.files += 1;
            timing.facts += candidate.facts.len();
            timing.read_duration += candidate.read_duration;
            timing.extract_duration += candidate.extract_duration;
        }
    }
}

struct ExtractionExecutor {
    pool: Option<rayon::ThreadPool>,
}

impl ExtractionExecutor {
    fn new(threads: &str) -> Result<Self> {
        let pool = if let Some(threads) = parse_threads(threads)? {
            Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .context("build extraction thread pool")?,
            )
        } else {
            None
        };

        Ok(Self { pool })
    }

    fn extract(
        &self,
        candidates: Vec<CandidateFile>,
        include_snippets: bool,
    ) -> Result<Vec<ExtractedCandidate>> {
        match &self.pool {
            Some(pool) => {
                pool.install(|| extract_candidates_parallel(candidates, include_snippets))
            }
            None => extract_candidates_parallel(candidates, include_snippets),
        }
    }
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
        pragma temp_store = file;
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
    let read_started_at = Instant::now();
    let content = read_candidate_content(&candidate.absolute_path)?;
    let read_duration = read_started_at.elapsed();

    let extract_started_at = Instant::now();
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
    let extract_duration = extract_started_at.elapsed();

    Ok(ExtractedCandidate {
        plugin: candidate.plugin,
        relative_path: candidate.relative_path,
        language: candidate.language,
        read_duration,
        extract_duration,
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
    candidates: Vec<CandidateFile>,
    walk_stats: WalkStats,
    include_snippets: bool,
) -> Result<IndexInsertResult> {
    let mut state = IndexInsertState {
        summary: BuildSummary {
            walk_plugins: walk_stats.plugins,
            files_seen: walk_stats.files_seen,
            candidates: candidates.len(),
            skipped_large: walk_stats.skipped_large,
            skipped_unsupported: walk_stats.skipped_unsupported,
            ..BuildSummary::default()
        },
        plugin_ids: HashMap::new(),
        surface_ids: HashMap::new(),
    };

    let extractor = ExtractionExecutor::new(&args.threads)?;
    let mut extraction_timings = ExtractionTimings::default();
    let mut extract_elapsed = Duration::default();
    let mut chunk = Vec::with_capacity(EXTRACTION_CHUNK_SIZE);

    for candidate in candidates {
        chunk.push(candidate);

        if chunk.len() == EXTRACTION_CHUNK_SIZE {
            let pending = std::mem::replace(&mut chunk, Vec::with_capacity(EXTRACTION_CHUNK_SIZE));
            process_candidate_chunk(
                transaction,
                args,
                &extractor,
                pending,
                include_snippets,
                &mut state,
                &mut extraction_timings,
                &mut extract_elapsed,
            )?;
        }
    }

    if !chunk.is_empty() {
        process_candidate_chunk(
            transaction,
            args,
            &extractor,
            chunk,
            include_snippets,
            &mut state,
            &mut extraction_timings,
            &mut extract_elapsed,
        )?;
    }

    state.summary.plugin_rows = state.plugin_ids.len();
    state.summary.surface_rows = state.surface_ids.len();
    state.summary.impact_rows = insert_impact_rows(transaction)?;

    Ok(IndexInsertResult {
        summary: state.summary,
        extraction_timings,
        extract_elapsed,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_candidate_chunk(
    transaction: &Transaction<'_>,
    args: &IndexArgs,
    extractor: &ExtractionExecutor,
    candidates: Vec<CandidateFile>,
    include_snippets: bool,
    state: &mut IndexInsertState,
    extraction_timings: &mut ExtractionTimings,
    extract_elapsed: &mut Duration,
) -> Result<()> {
    let extract_started_at = Instant::now();
    let extracted_candidates = extractor
        .extract(candidates, include_snippets)
        .context("extract candidate chunk")?;
    *extract_elapsed += extract_started_at.elapsed();
    extraction_timings.add_candidates(&extracted_candidates);

    insert_extracted_candidates(
        transaction,
        args,
        extracted_candidates,
        include_snippets,
        state,
    )
}

fn insert_extracted_candidates(
    transaction: &Transaction<'_>,
    args: &IndexArgs,
    candidates: Vec<ExtractedCandidate>,
    include_snippets: bool,
    state: &mut IndexInsertState,
) -> Result<()> {
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
        let plugin_id = if let Some(plugin_id) = state.plugin_ids.get(&plugin_key) {
            *plugin_id
        } else {
            insert_plugin.execute(params![
                &plugin_key.0,
                plugin_key.1.as_deref(),
                &plugin_key.2,
            ])?;
            let plugin_id = transaction.last_insert_rowid();
            state.plugin_ids.insert(plugin_key, plugin_id);
            plugin_id
        };

        insert_file.execute(params![
            plugin_id,
            path_to_string(&candidate.relative_path),
            Option::<Vec<u8>>::None,
        ])?;
        let file_id = transaction.last_insert_rowid();

        state.summary.facts_extracted += candidate.facts.len();

        for fact in candidate.facts {
            if fact.role != FactRole::Usage
                || !fact.confidence.include(args.include_low_confidence, false)
            {
                continue;
            }

            let surface_key = fact.surface.as_str().to_owned();
            let surface_id = if let Some(surface_id) = state.surface_ids.get(&surface_key) {
                *surface_id
            } else {
                insert_surface.execute(params![&surface_key, fact.kind.as_str()])?;
                let surface_id = transaction.last_insert_rowid();
                state.surface_ids.insert(surface_key, surface_id);
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

            state.summary.facts_stored += 1;
            state.summary.evidence_rows += 1;
        }
    }

    Ok(())
}

fn insert_impact_rows(transaction: &Transaction<'_>) -> Result<usize> {
    let rows = transaction.execute(
        "
        insert into impact (surface_id, plugin_id, usage_count, confidence)
        select surface_id, plugin_id, count(*), max(confidence)
        from evidence
        group by surface_id, plugin_id
        ",
        [],
    )?;

    Ok(rows)
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
    pattern: &str,
    include_low_confidence: bool,
    only_high_confidence: bool,
    max_evidence: usize,
) -> Result<QueryResult> {
    let query = SurfaceQuery::from_input(pattern);
    let min_confidence = if include_low_confidence { 1 } else { 2 };
    let require_high = if only_high_confidence { 1 } else { 0 };
    let surface_filter = query.sql_filter();
    let surface_value = query.sql_value();
    let indexed_plugins = indexed_plugin_count(connection)?;

    let count_sql = format!(
        "
        select count(distinct s.id), count(distinct e.plugin_id), count(*)
        from evidence e
        join surface s on s.id = e.surface_id
        where {surface_filter}
            and e.confidence >= ?2
            and (?3 = 0 or e.confidence = 3)
        "
    );
    let (matched_surfaces, affected_plugins, total_usages): (i64, i64, i64) = connection
        .query_row(
            &count_sql,
            params![surface_value, min_confidence, require_high],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    let mut surfaces = query_surface_totals(
        connection,
        surface_filter,
        surface_value,
        min_confidence,
        require_high,
    )?;

    let evidence_sql = format!(
        "
        select
            s.key,
            p.name,
            p.version,
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
        where {surface_filter}
            and e.confidence >= ?2
            and (?3 = 0 or e.confidence = 3)
        order by s.key asc, e.confidence desc, p.name asc, f.path asc, e.line asc, coalesce(e.column, 0) asc, e.usage_kind asc
        limit ?4
        "
    );

    let mut statement = connection.prepare(&evidence_sql)?;
    let max_evidence = i64::try_from(max_evidence).unwrap_or(i64::MAX);
    let evidence = statement
        .query_map(
            params![surface_value, min_confidence, require_high, max_evidence],
            |row| {
                let confidence: i64 = row.get(9)?;
                Ok(QueryEvidence {
                    surface: row.get(0)?,
                    plugin_name: row.get(1)?,
                    plugin_version: row.get(2)?,
                    plugin_folder: row.get(3)?,
                    file_path: row.get(4)?,
                    line: i64_to_usize(row.get(5)?),
                    column: row.get::<_, Option<i64>>(6)?.map(i64_to_usize),
                    usage_kind: row.get(7)?,
                    snippet: row.get(8)?,
                    confidence: Confidence::from_i64(confidence),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let surface_positions = surfaces
        .iter()
        .enumerate()
        .map(|(index, surface)| (surface.surface.clone(), index))
        .collect::<HashMap<_, _>>();

    for evidence in evidence {
        if let Some(index) = surface_positions.get(&evidence.surface) {
            surfaces[*index].evidence.push(evidence);
        }
    }

    Ok(QueryResult {
        query,
        matched_surfaces: i64_to_usize(matched_surfaces),
        affected_plugins: i64_to_usize(affected_plugins),
        total_usages: i64_to_usize(total_usages),
        indexed_plugins,
        elapsed: None,
        surfaces,
    })
}

fn query_surface_totals(
    connection: &Connection,
    surface_filter: &str,
    surface_value: &str,
    min_confidence: i64,
    require_high: i64,
) -> Result<Vec<QuerySurfaceResult>> {
    let surface_sql = format!(
        "
        select
            s.key,
            s.kind,
            count(*),
            count(distinct e.plugin_id),
            max(e.confidence)
        from evidence e
        join surface s on s.id = e.surface_id
        where {surface_filter}
            and e.confidence >= ?2
            and (?3 = 0 or e.confidence = 3)
        group by s.id, s.key, s.kind
        order by s.key asc
        "
    );

    let mut statement = connection.prepare(&surface_sql)?;
    let surfaces = statement
        .query_map(
            params![surface_value, min_confidence, require_high],
            |row| {
                let confidence: Option<i64> = row.get(4)?;
                Ok(QuerySurfaceResult {
                    surface: row.get(0)?,
                    kind: row.get(1)?,
                    usage_count: i64_to_usize(row.get(2)?),
                    affected_plugins: i64_to_usize(row.get(3)?),
                    confidence: Confidence::from_i64(confidence.unwrap_or(1)),
                    evidence: Vec::new(),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(surfaces)
}

fn indexed_plugin_count(connection: &Connection) -> Result<usize> {
    let count: i64 = connection
        .query_row("select count(*) from plugin", [], |row| row.get(0))
        .context("query indexed plugin count")?;
    Ok(i64_to_usize(count))
}

impl SurfaceQuery {
    fn from_input(input: &str) -> Self {
        if contains_wildcard(input) {
            return SurfaceQuery::Pattern {
                input: input.to_owned(),
                like_pattern: wildcard_to_like(input),
            };
        }

        if input.contains(':') {
            SurfaceQuery::Exact(input.to_owned())
        } else {
            SurfaceQuery::Pattern {
                input: input.to_owned(),
                like_pattern: format!("%{}%", escape_like(input)),
            }
        }
    }

    fn sql_filter(&self) -> &'static str {
        match self {
            SurfaceQuery::Exact(_) => "s.key = ?1",
            SurfaceQuery::Pattern { .. } => "s.key like ?1 escape '\\'",
        }
    }

    fn sql_value(&self) -> &str {
        match self {
            SurfaceQuery::Exact(input) => input,
            SurfaceQuery::Pattern { like_pattern, .. } => like_pattern,
        }
    }
}

fn contains_wildcard(input: &str) -> bool {
    input
        .chars()
        .any(|character| matches!(character, '*' | '?'))
}

fn wildcard_to_like(input: &str) -> String {
    let mut pattern = String::new();

    for character in input.chars() {
        match character {
            '*' => pattern.push('%'),
            '?' => pattern.push('_'),
            _ => push_escaped_like_char(&mut pattern, character),
        }
    }

    pattern
}

fn escape_like(input: &str) -> String {
    let mut pattern = String::new();

    for character in input.chars() {
        push_escaped_like_char(&mut pattern, character);
    }

    pattern
}

fn push_escaped_like_char(pattern: &mut String, character: char) {
    if matches!(character, '%' | '_' | '\\') {
        pattern.push('\\');
    }

    pattern.push(character);
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

fn print_index_timings(
    discover_elapsed: Duration,
    extract_elapsed: Duration,
    sqlite_elapsed: Duration,
    extraction_timings: &ExtractionTimings,
) {
    eprintln!(
        "Timings: discovery {}, extraction wall {}, SQLite write/index {}.",
        format_duration(discover_elapsed),
        format_duration(extract_elapsed),
        format_duration(sqlite_elapsed),
    );

    for language in [
        Language::Php,
        Language::Twig,
        Language::Xml,
        Language::Json,
        Language::Yaml,
        Language::Toml,
        Language::JavaScript,
        Language::TypeScript,
        Language::Vue,
    ] {
        let Some(timing) = extraction_timings.languages.get(&language) else {
            continue;
        };

        eprintln!(
            "  {:<10} files {:>7}, facts {:>8}, read {}, extractor accumulated {}.",
            language_label(language),
            timing.files,
            timing.facts,
            format_duration(timing.read_duration),
            format_duration(timing.extract_duration),
        );
    }
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

fn print_query_result(result: &QueryResult, options: ReportOptions) {
    let mut output = String::new();
    writeln!(output, "Shopware Impact Query").unwrap();
    let evidence_rows_shown = result.evidence_rows_shown();

    if result.total_usages == 0 {
        writeln!(output).unwrap();
        writeln!(output, "No evidence found.").unwrap();
    } else if evidence_rows_shown == 0 {
        writeln!(output).unwrap();
        writeln!(output, "Evidence output limited to 0 rows.").unwrap();
    } else {
        writeln!(output).unwrap();
        write_query_surface_results(&mut output, result, options);
        let hidden = result.total_usages.saturating_sub(evidence_rows_shown);
        if hidden > 0 {
            writeln!(output, "  ... {hidden} more evidence row(s) omitted").unwrap();
        }
    }

    write_query_summary(&mut output, result);
    print!("{output}");
}

fn write_query_surface_results(output: &mut String, result: &QueryResult, options: ReportOptions) {
    let mut wrote_surface = false;

    for surface in result
        .surfaces
        .iter()
        .filter(|surface| !surface.evidence.is_empty())
    {
        if wrote_surface {
            writeln!(output).unwrap();
        }

        write_query_surface_header(output, surface);
        let evidence = surface
            .evidence
            .iter()
            .map(|evidence| EvidenceDisplay {
                plugin: evidence.plugin_label(),
                file: evidence.location(),
                usage: evidence.usage_kind.clone(),
                confidence: evidence.confidence,
                source_url: evidence.github_url(),
                snippet: evidence.snippet.clone(),
            })
            .collect::<Vec<_>>();
        write_numbered_evidence_entries(output, &evidence, options);
        wrote_surface = true;
    }
}

fn write_query_surface_header(output: &mut String, surface: &QuerySurfaceResult) {
    let rows = vec![
        ("surface kind", surface.kind.clone()),
        ("confidence", surface.confidence.as_str().to_owned()),
        ("affected plugins", surface.affected_plugins.to_string()),
        ("usages", surface.usage_count.to_string()),
    ];
    write_dashed_block(output, &surface.surface, &rows);
}

fn write_query_summary(output: &mut String, result: &QueryResult) {
    writeln!(output).unwrap();
    writeln!(output, "{SEPARATOR}").unwrap();
    writeln!(output, "Summary").unwrap();

    let mut rows = Vec::new();
    match &result.query {
        SurfaceQuery::Exact(surface) => rows.push(("Surface", surface.clone())),
        SurfaceQuery::Pattern { input, .. } => {
            rows.push(("Query", input.clone()));
            rows.push(("Matching surfaces", result.matched_surfaces.to_string()));
        }
    }
    if let Some(elapsed) = result.elapsed {
        rows.push(("Runtime", format_duration(elapsed)));
    }
    rows.extend([
        ("Usages", result.total_usages.to_string()),
        (
            "Evidence rows shown",
            result.evidence_rows_shown().to_string(),
        ),
        (
            "Affected plugins",
            affected_plugins_summary(result.affected_plugins, Some(result.indexed_plugins)),
        ),
    ]);

    write_aligned_key_values(output, "", &rows);
    writeln!(output, "{SEPARATOR}").unwrap();
}

impl QueryEvidence {
    fn plugin_label(&self) -> String {
        match self.plugin_version.as_deref() {
            Some(version) if !version.is_empty() => format!("{}@{version}", self.plugin_name),
            _ => self.plugin_name.clone(),
        }
    }

    fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.line)
    }

    fn github_url(&self) -> String {
        store_plugin_mirror_url(&self.plugin_folder, Path::new(&self.file_path), self.line)
    }
}

impl QueryResult {
    fn evidence_rows_shown(&self) -> usize {
        self.surfaces
            .iter()
            .map(|surface| surface.evidence.len())
            .sum()
    }

    #[cfg(test)]
    fn evidence_rows(&self) -> Vec<&QueryEvidence> {
        self.surfaces
            .iter()
            .flat_map(|surface| surface.evidence.iter())
            .collect()
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

fn language_label(language: Language) -> &'static str {
    match language {
        Language::Php => "php",
        Language::Twig => "twig",
        Language::Xml => "xml",
        Language::Json => "json",
        Language::Yaml => "yaml",
        Language::Toml => "toml",
        Language::JavaScript => "js",
        Language::TypeScript => "ts",
        Language::Vue => "vue",
        Language::Unknown => "unknown",
    }
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
        assert_eq!(default.evidence_rows_shown(), 2);
        assert!(
            default
                .evidence_rows()
                .iter()
                .all(|evidence| evidence.confidence != Confidence::Low)
        );

        let include_low = query_surface(&connection, "service:id:shopware.foo", true, false, 1)
            .expect("query with low confidence");
        assert_eq!(include_low.affected_plugins, 2);
        assert_eq!(include_low.total_usages, 4);
        assert_eq!(include_low.evidence_rows_shown(), 1);

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
        assert_eq!(only_high.evidence_rows()[0].confidence, Confidence::High);
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
