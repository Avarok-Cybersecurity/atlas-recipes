// SPDX-License-Identifier: AGPL-3.0-only

//! `atlasctl bench` — run a certification gate on a paired node.
//!
//! Every subcommand takes `--json` and then writes exactly one JSON document
//! to stdout (NDJSON, one event per line, for `attach` and `run`) and nothing
//! else there, so a caller such as `spark bench certify` parses stdout and
//! never the log. Failures under `--json` are an `ErrorObj` on stdout with a
//! distinct exit code per class; see [`crate::commands::bench::exit`].

use clap::{Args, Subcommand};

/// Bench subcommands.
#[derive(Subcommand, Debug)]
pub enum BenchCmd {
    /// What each node can run: hardware, repo, signer, queue, built commits.
    Nodes(NodesArgs),
    /// Submit a gate at a commit; prints the job id. Idempotent by key.
    Submit(SubmitArgs),
    /// Follow a job's events, re-attaching from the last seq on a dropped link.
    Attach(AttachArgs),
    /// The node's jobs, or one job.
    Status(StatusArgs),
    /// Stop a job. Idempotent.
    Cancel(JobArgs),
    /// What a job wrote.
    Artifacts(JobArgs),
    /// Download a job's artifacts, verified, into a directory.
    Fetch(FetchArgs),
    /// Submit, follow and fetch in one command.
    Run(RunArgs),
}

/// Output shape.
#[derive(Args, Debug, Clone, Copy)]
pub struct OutputArgs {
    /// One JSON document on stdout (NDJSON for attach/run); errors too.
    #[arg(long)]
    pub json: bool,
}

/// One node address.
///
/// `ip[:port]`, `[v6]:port`, `host.local[:port]`, `dns.name[:port]`. Port
/// omitted → the default peer port.
#[derive(Args, Debug)]
pub struct NodeArg {
    #[arg(value_name = "ADDR")]
    pub node: String,
}

#[derive(Args, Debug)]
pub struct NodesArgs {
    /// Node addresses, comma-separated or repeated.
    #[arg(value_name = "ADDR", required = true, value_delimiter = ',')]
    pub nodes: Vec<String>,
    #[command(flatten)]
    pub out: OutputArgs,
}

/// What to run.
#[derive(Args, Debug)]
pub struct JobSpecArgs {
    /// The 40-hex commit to build and run.
    #[arg(long, value_name = "SHA")]
    pub sha: String,
    /// The gate's benchmark id, e.g. `decode-floor`.
    #[arg(long, value_name = "ID")]
    pub gate: String,
    /// A benchmark parameter, `key=value`; repeatable.
    #[arg(long = "param", value_name = "K=V")]
    pub params: Vec<String>,
    /// Serve this checkpoint instead of the gate's default.
    #[arg(long, value_name = "NAME")]
    pub checkpoint: Option<String>,
    /// The box class the record must claim; refused if the node is another.
    #[arg(long, value_name = "CLASS")]
    pub hardware: Option<String>,
    /// Cap on the run phase, seconds.
    #[arg(long, value_name = "SECS")]
    pub max_run_s: Option<u32>,
    /// A note stored with the job.
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,
    /// Idempotency key: resubmitting it returns the same job. Derived from
    /// the sha, gate and params when omitted, so a repeat of the same command
    /// is a repeat, not a second job.
    #[arg(long, value_name = "KEY")]
    pub job_key: Option<String>,
}

#[derive(Args, Debug)]
pub struct SubmitArgs {
    #[command(flatten)]
    pub node: NodeArg,
    #[command(flatten)]
    pub spec: JobSpecArgs,
    #[command(flatten)]
    pub out: OutputArgs,
}

/// How long to keep re-attaching.
#[derive(Args, Debug, Clone, Copy)]
pub struct ReconnectArgs {
    /// Keep re-attaching after a dropped link for this many seconds; 0 gives
    /// up on the first drop. The default is a day: longer than any gate, so
    /// an unattended follow outlives a network blip but not a dead node.
    #[arg(long, value_name = "SECS", default_value_t = 86_400)]
    pub reconnect_for: u64,
}

#[derive(Args, Debug)]
pub struct AttachArgs {
    #[command(flatten)]
    pub node: NodeArg,
    /// The job id from `submit`.
    #[arg(value_name = "JOB")]
    pub job: String,
    /// First seq to receive; 1 replays the job from its start.
    #[arg(long, value_name = "SEQ", default_value_t = 1)]
    pub from_seq: u64,
    #[command(flatten)]
    pub reconnect: ReconnectArgs,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[command(flatten)]
    pub node: NodeArg,
    /// One job rather than the list.
    #[arg(long, value_name = "JOB")]
    pub job: Option<String>,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args, Debug)]
pub struct JobArgs {
    #[command(flatten)]
    pub node: NodeArg,
    #[arg(value_name = "JOB")]
    pub job: String,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args, Debug)]
pub struct FetchArgs {
    #[command(flatten)]
    pub node: NodeArg,
    #[arg(value_name = "JOB")]
    pub job: String,
    /// Directory to write into; each artifact lands at its repo-relative path.
    #[arg(long, value_name = "DIR")]
    pub out_dir: std::path::PathBuf,
    #[command(flatten)]
    pub out: OutputArgs,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[command(flatten)]
    pub node: NodeArg,
    #[command(flatten)]
    pub spec: JobSpecArgs,
    /// Directory to write the artifacts into once the job is done.
    #[arg(long, value_name = "DIR")]
    pub out_dir: std::path::PathBuf,
    #[command(flatten)]
    pub reconnect: ReconnectArgs,
    #[command(flatten)]
    pub out: OutputArgs,
}
