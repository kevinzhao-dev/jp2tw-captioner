# rusty

A personal collection of Rust tools and experiments.

This repository hosts multiple standalone tools, each in its own subfolder.

## Suggested learning projects

If you're looking for ideas that balance practicality with opportunities to
practice core Rust concepts, here are several project suggestions:

1. **CLI log analyzer** – Parse structured log files (JSON, CSV, or syslog) and
   emit aggregated statistics. This gives you practice with error handling,
   iterators, and third-party crates such as `serde`.
   - *Suggested steps*: (a) prototype a parser for one log format, (b) define a
     data model and aggregations to compute (counts, percentiles, histograms),
     (c) add CLI arguments for input paths, filters, and output format, (d)
     implement tests using fixture log files, and (e) document usage examples in
     the README.
2. **Static site generator** – Convert Markdown content into HTML files. This is
   a great way to experiment with templating engines, file I/O, and testing.
   - *Suggested steps*: (a) load Markdown files from a content directory, (b)
     render them to HTML with front matter metadata, (c) apply a template engine
     for consistent layouts, (d) copy static assets into an output folder, and
     (e) add watch mode or incremental rebuilds once the basics work.
3. **Task runner** – Implement a lightweight alternative to `make` that reads a
   configuration file and executes tasks in dependency order, introducing you to
   graph algorithms and concurrency primitives like channels.
   - *Suggested steps*: (a) design the configuration schema (YAML/TOML), (b)
     parse tasks and build a dependency graph, (c) perform topological sort with
     cycle detection, (d) execute tasks sequentially then experiment with
     parallel execution, and (e) surface summaries and exit codes clearly.
4. **File synchronization utility** – Mirror a local directory to another path
   while detecting changes efficiently. This reinforces your understanding of
   lifetimes, ownership, and async runtimes.
   - *Suggested steps*: (a) traverse the source directory and snapshot file
     metadata, (b) detect additions/changes via hashing or timestamps, (c)
     perform copy/delete operations while preserving permissions, (d) add a
     daemon/watch mode using `notify` or async streams, and (e) expose dry-run
     and conflict resolution options.
5. **Terminal dashboard** – Build an interactive TUI that shows system metrics
   by combining crates like `tui` and `sysinfo`, which is excellent practice for
   structuring stateful applications.
   - *Suggested steps*: (a) render a static layout with panes, (b) poll system
     metrics (CPU, memory, disk) on an interval, (c) wire keyboard shortcuts for
     navigation, (d) maintain shared state with channels or `Arc<Mutex<_>>`, and
     (e) add export/logging of collected metrics for later analysis.

Each of these tools can start simple and grow in complexity as you explore more
advanced Rust techniques.

## AI agent and LLM-oriented mini-projects

Looking to experiment with smaller-scope agentic tooling that still exercises
Rust's ecosystem around HTTP, async runtimes, and embeddings? The ideas below
include lightweight specifications and practical hints so you can move from
brainstorming to implementation faster.

### Embeddings-powered snippet search

**Core requirements**
- Recursively crawl a source directory for Markdown, text, or code files.
- Chunk file contents deterministically (for example, paragraph or function
  boundaries) and generate embeddings via an external API.
- Store the embeddings locally (e.g., in an `sqlite` table or serialized to
  disk) alongside metadata such as file path and line range.
- Provide a CLI command (`search <query>`) that retrieves the top-k similar
  chunks and displays snippets with context and score.

**Implementation hints**
- Start with crates like `walkdir` for file discovery and `serde_json` or `csv`
  for serializing embedding stores.
- Use a cosine similarity helper from crates such as `ndarray`, `linfa`, or
  `qdrant-client`.
- Cache API responses by hashing chunk content; skip regeneration when a hash
  matches a previous run.
- Add a `--refresh` flag that forces re-embedding changed files.

### Single-file RAG assistant

**Core requirements**
- Accept a path (or glob) to the local knowledge base and chunk it into small
  passages.
- Generate embeddings and persist them to disk so the assistant can run without
  reprocessing each time.
- Implement a conversational loop that performs retrieval against the stored
  embeddings before calling an LLM chat completion API.
- Expose configuration for API keys, model names, temperature, and top-k
  results through a TOML or YAML config file.

**Implementation hints**
- Model your pipeline as a series of pure functions (`load -> chunk -> embed ->
  retrieve -> respond`) to simplify testing.
- Use `clap` or `argh` for CLI parsing and `tokio` for async API calls.
- Employ approximate nearest neighbour crates (`hnsw_rs`, `vectordb`) or a
  simple brute-force cosine similarity to keep the scope manageable.
- Implement graceful fallbacks when the API fails (retry with exponential backoff
  or surface cached answers).

### Prompt template runner

**Core requirements**
- Load a template file containing named placeholders (e.g., `{{title}}`).
- Ingest structured inputs from CSV or JSON and render prompts for each row or
  object.
- Stream responses from the LLM provider to stdout and optionally write them to
  a results file.
- Support rate limiting (e.g., configurable delay between requests) and partial
  resume if execution is interrupted.

**Implementation hints**
- Reuse templating crates like `tera` or `handlebars` for prompt rendering.
- Parse CSV data with `csv` crate and leverage `serde` for JSON.
- Wrap the LLM call in a small worker that implements retry with jitter to stay
  under rate limits.
- Keep run metadata (timestamps, input IDs, response tokens) so you can resume
  from the last successful record.

### Code-execution assistant

**Core requirements**
- Accept natural-language input (e.g., `"list orphaned docker images"`) and
  produce a candidate shell command via an LLM completion.
- Display the generated command, ask the user to confirm/deny, and optionally
  offer an edit prompt.
- Execute confirmed commands in a sandboxed subprocess and capture stdout/stderr
  for display.
- Maintain a history log (JSON or SQLite) of prompts, generated commands,
  approvals, and outputs for auditing.

**Implementation hints**
- Use `tokio::process::Command` or `duct` for command execution while enforcing
  timeouts.
- Provide a `--dry-run` flag that skips execution but still logs the interaction.
- Harden the agent by restricting the environment variables passed to the child
  process and optionally whitelisting allowed commands.
- Consider integrating shell completions or `fzf` to help the user select common
  commands safely.

### LLM evaluation harness

**Core requirements**
- Load a dataset of prompts and expected attributes (e.g., desired answer or
  rubric tags).
- Execute the dataset against multiple providers or model versions with uniform
  APIs.
- Persist raw responses and derived metrics (accuracy, BLEU/BERTScore, latency)
  for later analysis.
- Expose a summary command that prints tables or writes Markdown reports.

**Implementation hints**
- Model providers behind a trait (`ModelProvider`) so you can swap in mocked
  implementations for tests.
- Keep datasets in a versioned folder and reference them via config to encourage
  reproducibility.
- Use `polars` or `serde` + `csv` to aggregate metrics efficiently.
- Add quick sanity checks (e.g., verifying API keys, validating dataset schema)
  before running the full evaluation suite.

These projects intentionally limit their scope while still letting you explore
how Rust can orchestrate LLM-powered workflows.
