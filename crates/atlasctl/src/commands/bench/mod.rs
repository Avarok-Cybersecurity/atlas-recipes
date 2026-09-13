// SPDX-License-Identifier: AGPL-3.0-only

//! `atlasctl bench` — run a certification gate on a paired node.
//!
//! Each subcommand is one conversation over the pinned peer channel, driven
//! by [`Session`]. Under `--json`, stdout carries exactly one document (one
//! event per line for `attach`/`run`) so a script — `spark bench certify`
//! is the one this exists for — parses stdout and reads the exit code; the
//! human rendering goes to stdout only without `--json`, and progress chatter
//! always to stderr.

pub mod address;
pub mod exit;
pub mod fetch;
pub mod follow;
pub mod render;
pub mod session;

use crate::cli::bench_args::{
    AttachArgs, FetchArgs, JobArgs, JobSpecArgs, NodesArgs, OutputArgs, RunArgs, StatusArgs,
    SubmitArgs,
};
use address::Target;
use exit::BenchError;
use follow::Reconnect;
use serde::Serialize;
use session::{Reached, Session};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

use atlasctl_protocol::msg::bench::{GateId, JobId, JobKey, Sha};
use atlasctl_protocol::msg::{BenchEvent, BenchRep, BenchReq};

type Out<T> = Result<T, BenchError>;

fn print_json<T: Serialize>(v: &T) -> Out<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, v).map_err(|e| BenchError::io(e.to_string()))?;
    out.write_all(b"\n")
        .map_err(|e| BenchError::io(e.to_string()))
}

fn mdns() -> Option<Box<dyn atlasctl_agent::discovery::DiscoveryBrowser>> {
    atlasctl_agent::discovery::mdns::MdnsDiscovery::new()
        .ok()
        .map(|d| Box::new(d) as Box<dyn atlasctl_agent::discovery::DiscoveryBrowser>)
}

fn resolve(target: &Target) -> Out<Vec<std::net::SocketAddr>> {
    // The browser is only started for a `.local` name the resolver could not
    // answer; on a locked-down network it may not start at all, and that is
    // just "no fallback".
    let browser = target.is_mdns_name().then(mdns).flatten();
    target.resolve(browser.as_deref())
}

fn job_id(s: &str) -> Out<JobId> {
    JobId::parse(s).map_err(|e| BenchError::bad_args(format!("job id {s:?}: {e}")))
}

/// What `submit` sends, built from the arguments and checked here so the
/// node never sees a request this side already knows it would refuse.
pub fn submit_request(spec: &JobSpecArgs) -> Out<BenchReq> {
    let sha = Sha::parse(&spec.sha).map_err(|e| BenchError::bad_args(format!("--sha: {e}")))?;
    let gate =
        GateId::parse(&spec.gate).map_err(|e| BenchError::bad_args(format!("--gate: {e}")))?;
    let mut params = BTreeMap::new();
    for p in &spec.params {
        let Some((k, v)) = p.split_once('=') else {
            return Err(BenchError::bad_args(format!(
                "--param {p:?} is not key=value"
            )));
        };
        if k.is_empty() {
            return Err(BenchError::bad_args(format!(
                "--param {p:?} has an empty key"
            )));
        }
        if params.insert(k.to_owned(), v.to_owned()).is_some() {
            return Err(BenchError::bad_args(format!("--param {k} given twice")));
        }
    }
    let job_key = match &spec.job_key {
        Some(k) => JobKey::parse(k).map_err(|e| BenchError::bad_args(format!("--job-key: {e}")))?,
        None => derived_key(&sha, &gate, &params),
    };
    Ok(BenchReq::Submit {
        job_key,
        sha,
        gate,
        params,
        checkpoint: spec.checkpoint.clone(),
        hardware: spec.hardware.clone(),
        max_run_s: spec.max_run_s,
        note: spec.note.clone(),
    })
}

/// `cli-<sha12>-<gate>-<params8>`: the same command is the same key.
#[must_use]
pub fn derived_key(sha: &Sha, gate: &GateId, params: &BTreeMap<String, String>) -> JobKey {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (k, v) in params {
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
        h.update(b"\n");
    }
    let p = hex::encode(h.finalize());
    // 4 + 12 + 1 + gate(≤ 64) + 1 + 8 can exceed the 64-byte key; trim the
    // gate, which is the only variable-length part, keeping it recognisable.
    let gate_part: String = gate.as_str().chars().take(38).collect();
    let key = format!("cli-{}-{gate_part}-{}", &sha.as_str()[..12], &p[..8]);
    JobKey::parse(&key).unwrap_or_else(|e| unreachable!("derived key {key:?} invalid: {e}"))
}

#[derive(Serialize)]
struct NodeRow<'a> {
    node: &'a str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    info: Option<Box<atlasctl_protocol::msg::BenchNodeInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<exit::ErrorObj>,
}

/// `bench nodes A,B,C`: every node is asked; the exit code is the worst.
///
/// # Errors
/// The first node's error when any node failed, after all are reported.
pub fn nodes(a: &NodesArgs) -> Out<()> {
    let targets = address::parse_all(&a.nodes)?;
    let session = Session::open()?;
    let mut rows = Vec::with_capacity(targets.len());
    let mut first_err: Option<BenchError> = None;
    for t in &targets {
        let row = match resolve(t).and_then(|addrs| session.request(t, &addrs, &BenchReq::NodeInfo))
        {
            Ok((_, BenchRep::NodeInfo { info })) => NodeRow {
                node: &t.given,
                ok: true,
                info: Some(info),
                error: None,
            },
            Ok((_, other)) => {
                let e = BenchError::unexpected(&t.given, &other);
                let obj = (*e.obj).clone();
                first_err.get_or_insert(e);
                NodeRow {
                    node: &t.given,
                    ok: false,
                    info: None,
                    error: Some(obj),
                }
            }
            Err(e) => {
                let obj = (*e.obj).clone();
                first_err.get_or_insert(e);
                NodeRow {
                    node: &t.given,
                    ok: false,
                    info: None,
                    error: Some(obj),
                }
            }
        };
        rows.push(row);
    }
    if a.out.json {
        print_json(&rows)?;
    } else {
        for r in &rows {
            match (&r.info, &r.error) {
                (Some(info), _) => print!("{}", render::node_block(r.node, info)),
                (None, Some(e)) => println!("{}  ERROR {}: {}", r.node, e.code, e.message),
                (None, None) => {}
            }
        }
    }
    // Under --json the rows already carry each error; the exit code still
    // says "not all ok" so a caller need not scan them to know.
    match first_err {
        Some(mut e) => {
            e.reported = true;
            Err(e)
        }
        None => Ok(()),
    }
}

#[derive(Serialize)]
struct Submitted<'a> {
    node: &'a str,
    node_id: String,
    job_id: JobId,
    job_key: JobKey,
    existing: bool,
    state: atlasctl_protocol::msg::bench::JobState,
    position: u32,
}

fn do_submit<'a>(
    session: &Session,
    t: &'a Target,
    spec: &JobSpecArgs,
) -> Out<(Reached, Submitted<'a>)> {
    let req = submit_request(spec)?;
    let BenchReq::Submit { job_key, .. } = &req else {
        unreachable!("submit_request builds Submit")
    };
    let addrs = resolve(t)?;
    let (reached, rep) = session.request(t, &addrs, &req)?;
    match rep {
        BenchRep::Accepted {
            job,
            existing,
            state,
            position,
        } => Ok((
            reached,
            Submitted {
                node: &t.given,
                node_id: reached.id.to_string(),
                job_id: job,
                job_key: job_key.clone(),
                existing,
                state,
                position,
            },
        )),
        other => Err(BenchError::unexpected(&t.given, &other)),
    }
}

/// `bench submit`.
///
/// # Errors
/// Bad arguments, a refusal, or a link failure.
pub fn submit(a: &SubmitArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let session = Session::open()?;
    let (_, s) = do_submit(&session, &t, &a.spec)?;
    if a.out.json {
        print_json(&s)
    } else {
        println!(
            "{} {}{} on {} ({}) — {:?}, position {}",
            s.job_id,
            s.job_key,
            if s.existing { " (existing)" } else { "" },
            t.given,
            s.node_id,
            s.state,
            s.position
        );
        Ok(())
    }
}

fn emitter(out: OutputArgs) -> impl FnMut(&BenchEvent) {
    move |ev: &BenchEvent| {
        if out.json {
            // A failed stdout is the caller going away; stopping here is the
            // right outcome and the job continues on the node regardless.
            if print_json(ev).is_err() {
                std::process::exit(exit::Code::StreamLost as i32);
            }
        } else {
            for line in render::event_lines(ev) {
                println!("{line}");
            }
        }
    }
}

fn finish(reached: Reached, followed: &follow::Followed) -> Out<()> {
    if followed.outcome.passed() {
        Ok(())
    } else {
        Err(BenchError::outcome(reached.id, &followed.outcome))
    }
}

/// `bench attach`.
///
/// # Errors
/// The job's failure as its exit code, a lost stream, or a refusal.
pub fn attach(a: &AttachArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let job = job_id(&a.job)?;
    let session = Session::open()?;
    let addrs = resolve(&t)?;
    // Which address answers is decided by a cheap request first, so the
    // follow loop re-attaches to the one address that is known to work.
    let (reached, _) = session.request(
        &t,
        &addrs,
        &BenchReq::Status {
            job: Some(job.clone()),
        },
    )?;
    let followed = follow::follow(
        &session,
        &t,
        reached.addr,
        &job,
        a.from_seq,
        Reconnect {
            budget: Duration::from_secs(a.reconnect.reconnect_for),
        },
        &mut emitter(a.out),
    )?;
    finish(reached, &followed)
}

/// `bench status`.
///
/// # Errors
/// A refusal or a link failure.
pub fn status(a: &StatusArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let job = a.job.as_deref().map(job_id).transpose()?;
    let session = Session::open()?;
    let addrs = resolve(&t)?;
    let (_, rep) = session.request(&t, &addrs, &BenchReq::Status { job })?;
    let BenchRep::Status { jobs } = rep else {
        return Err(BenchError::unexpected(&t.given, &rep));
    };
    if a.out.json {
        print_json(&jobs)
    } else {
        print!("{}", render::status_table(&jobs));
        Ok(())
    }
}

/// `bench cancel`. Idempotent: cancelling a finished job reports its state.
///
/// # Errors
/// A refusal (unknown job) or a link failure.
pub fn cancel(a: &JobArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let job = job_id(&a.job)?;
    let session = Session::open()?;
    let addrs = resolve(&t)?;
    let (reached, rep) = session.request(&t, &addrs, &BenchReq::Cancel { job })?;
    let BenchRep::Cancelled { job, state } = rep else {
        return Err(BenchError::unexpected(&t.given, &rep));
    };
    #[derive(Serialize)]
    struct Cancelled<'a> {
        node: &'a str,
        node_id: String,
        job: JobId,
        state: atlasctl_protocol::msg::bench::JobState,
    }
    let c = Cancelled {
        node: &t.given,
        node_id: reached.id.to_string(),
        job,
        state,
    };
    if a.out.json {
        print_json(&c)
    } else {
        println!("{} on {}: {:?}", c.job, t.given, c.state);
        Ok(())
    }
}

/// `bench artifacts`.
///
/// # Errors
/// A refusal or a link failure.
pub fn artifacts(a: &JobArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let job = job_id(&a.job)?;
    let session = Session::open()?;
    let addrs = resolve(&t)?;
    let (_, items) = fetch::list(&session, &t, &addrs, &job)?;
    if a.out.json {
        print_json(&items)
    } else {
        for m in &items {
            println!(
                "{:<10}  {:>10}  {}  {}",
                format!("{:?}", m.kind).to_ascii_lowercase(),
                m.bytes,
                &m.sha256[..12],
                m.relative_path
            );
        }
        Ok(())
    }
}

/// `bench fetch`.
///
/// # Errors
/// A refusal, a link failure, a verification failure, or an existing file.
pub fn fetch(a: &FetchArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let job = job_id(&a.job)?;
    let session = Session::open()?;
    let addrs = resolve(&t)?;
    let (_, files) = fetch::fetch_all(&session, &t, &addrs, &job, &a.out_dir)?;
    if a.out.json {
        print_json(&files)
    } else {
        for f in &files {
            println!("{}  {} bytes", f.path.display(), f.bytes);
        }
        Ok(())
    }
}

/// Run one subcommand, tagging any failure with its `--json` so `main`
/// renders it the way the caller asked.
///
/// # Errors
/// The subcommand's.
pub fn dispatch(cmd: &crate::cli::BenchCmd) -> Out<()> {
    use crate::cli::BenchCmd as C;
    let (json, r) = match cmd {
        C::Nodes(a) => (a.out.json, nodes(a)),
        C::Submit(a) => (a.out.json, submit(a)),
        C::Attach(a) => (a.out.json, attach(a)),
        C::Status(a) => (a.out.json, status(a)),
        C::Cancel(a) => (a.out.json, cancel(a)),
        C::Artifacts(a) => (a.out.json, artifacts(a)),
        C::Fetch(a) => (a.out.json, fetch(a)),
        C::Run(a) => (a.out.json, run(a)),
    };
    r.map_err(|mut e| {
        e.json = json;
        e
    })
}

/// `bench run`: submit, follow, fetch.
///
/// # Errors
/// Any of the three steps'; the job's own failure is its exit code.
pub fn run(a: &RunArgs) -> Out<()> {
    let t = Target::parse(&a.node.node)?;
    let session = Session::open()?;
    let (reached, s) = do_submit(&session, &t, &a.spec)?;
    eprintln!(
        "{} {}{} on {} ({})",
        s.job_id,
        s.job_key,
        if s.existing { " (existing)" } else { "" },
        t.given,
        reached.id.short()
    );
    let job = s.job_id.clone();
    let followed = follow::follow(
        &session,
        &t,
        reached.addr,
        &job,
        1,
        Reconnect {
            budget: Duration::from_secs(a.reconnect.reconnect_for),
        },
        &mut emitter(a.out),
    )?;
    let (_, files) = fetch::fetch_all(&session, &t, &[reached.addr], &job, &a.out_dir)?;
    #[derive(Serialize)]
    struct Ran<'a> {
        job_id: &'a JobId,
        outcome: &'a atlasctl_protocol::msg::bench_event::Outcome,
        passed: bool,
        files: &'a [fetch::Fetched],
    }
    if a.out.json {
        print_json(&Ran {
            job_id: &job,
            outcome: &followed.outcome,
            passed: followed.outcome.passed(),
            files: &files,
        })?;
    } else {
        for f in &files {
            println!("{}  {} bytes", f.path.display(), f.bytes);
        }
    }
    finish(reached, &followed)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
