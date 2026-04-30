# Chunk 06: SQLite Index And Query

## Goal

Persist extracted plugin usage facts into a SQLite inverted index and support exact surface queries.

## Scope

- SQLite schema creation.
- Full rebuild into a new SQLite file.
- Bulk insert path.
- Aggregation by surface/plugin.
- Evidence storage.
- Exact query command.
- `--no-snippets`.
- `--include-low-confidence`.

## Schema

```sql
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
    hash blob
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
    primary key (surface_id, plugin_id)
);

create table evidence (
    surface_id integer not null,
    plugin_id integer not null,
    file_id integer not null,
    line integer not null,
    column integer,
    usage_kind text not null,
    snippet text,
    confidence integer not null
);

create index evidence_surface_plugin on evidence(surface_id, plugin_id);
create index impact_surface on impact(surface_id);
```

## Build Rules

- Create a new file for each full rebuild.
- Refuse to overwrite existing output unless `--force` is set.
- Use one transaction.
- Batch inserts.
- Create indexes after bulk inserts.
- Store snippets by default.
- Drop low-confidence facts unless `--include-low-confidence` is set.

## Query Output

```text
Surface: twig:block:storefront_page_product_detail_buy
Affected plugins: 2
Usages: 3

PluginA  src/Resources/views/storefront/page/product-detail/index.html.twig:7  block-override  high
PluginB  src/Foo.php:42                                                        string-ref      medium
```

## Implementation Notes

- Use numeric IDs internally.
- Keep storage code independent from CLI formatting.
- Add a schema version table.
- Consider storing normalized index metadata:
  - created_at
  - corpus root
  - fact count
  - plugin count
  - skipped file count
  - include_low_confidence
  - snippets_enabled

## Acceptance

- `sw-impact index` writes a SQLite file from fixture plugins.
- `sw-impact query` returns expected fixture evidence.
- Existing output file is protected unless `--force` is used.
- `--no-snippets` omits snippets.
- `--include-low-confidence` changes stored fact count in tests.
- Basic performance is measured on a corpus sample.

## Out Of Scope

- Compact read-only custom index.
- CI threshold behavior.
- JSON report output.
