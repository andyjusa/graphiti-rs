#!/usr/bin/env python3
"""Compare two pinned Rust images with the unchanged paired Pi evaluator.

Stage this file and bench-paired-pi.py under ROOT/scripts. ROOT must be an
owned mode-0700 /home/pi4-3/graphiti-rust-bench-20261016-<suffix> directory;
its OWNED file must contain graphiti-rust-pi-20261016. Run only on Linux ARM64.
Each of six executions gets a prospective .autoresearch manifest and ledger.
This coordinator never changes the evaluator, production services, or images.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shlex
import signal
import stat
import sys
import uuid

MARKER = "graphiti-rust-pi-20261016"
EVALUATOR_SHA256 = "d9235decb425a380eacced02238c42f7ee03bc49a8e371e23b4dbf3418317574"
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("paired_pi", HERE / "bench-paired-pi.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


class Experiment(base.Experiment):
    def __init__(self, root, builds, command):
        self.builds = builds
        self.command = command
        self.active = None
        self.protected = {
            f"scripts/{name}": digest(root / "scripts" / name)
            for name in ("bench-paired-pi.py", "bench-rust-pi.py")
        }
        require(self.protected["scripts/bench-paired-pi.py"] == EVALUATOR_SHA256,
                "protected paired evaluator differs from the preserved benchmark")
        for kind, build in builds.items():
            path = root / "source-manifests" / f"{kind}.json"
            require(not path.is_symlink() and path.resolve().is_relative_to(root),
                    f"{kind} source manifest must be inside the owned root")
            source = json.loads(path.read_text())
            require(source["source_commit"] == build["source_commit"],
                    f"{kind} source manifest commit disagrees with supplied commit")
            binary_hash = source["files"]["bin/graphiti-rs"]
            require(re.fullmatch(r"[0-9a-f]{64}", binary_hash), f"{kind} manifest binary hash is invalid")
            relative = str(path.relative_to(root))
            self.protected[relative] = digest(path)
            build["source_build_manifest"] = {"path": relative, "sha256": digest(path)}
            build["expected_binary_sha256"] = binary_hash
        require(not (root / "report.json").exists() and not (root / ".autoresearch").exists(),
                "use a fresh owned root; an existing execution must be reconciled first")
        super().__init__(root)
        self.notebook = root / ".autoresearch"
        self.notebook.mkdir(mode=0o700)
        (self.notebook / "runs").mkdir(mode=0o700)
        self.comparison_group = MARKER + "-" + uuid.uuid4().hex
        self.runs = {}
        for trial in range(3):
            for kind in ("baseline", "candidate"):
                run_id = f"{kind}-{trial + 1}-{uuid.uuid4().hex}"
                directory = self.notebook / "runs" / run_id
                directory.mkdir(mode=0o700)
                self.runs[trial, kind] = {"run_id": run_id, "directory": directory,
                                         "kind": kind, "trial": trial}
        self.report.pop("python_image")
        self.report.pop("rust_image")
        self.report.pop("rust_source_commit")
        self.report.update(marker=MARKER, support_image=base.PYTHON_IMAGE,
                           builds=builds, comparison_group=self.comparison_group,
                           protected_path_sha256=self.protected)
        self.report["design"]["orders"] = [
            ["baseline", "candidate"], ["candidate", "baseline"], ["baseline", "candidate"]]
        self.smoke_command = "python3 -m py_compile scripts/bench-rust-pi.py scripts/bench-paired-pi.py"
        self.verify_command = shlex.join(["python3", "-c",
            "import json; r=json.load(open('report.json')); "
            "assert r['status']=='completed' and len(r['runs'])==6 and not r['cleanup_errors']; "
            "assert sum(w['successes'] for run in r['runs'] for w in run['workloads'].values())==1800; "
            "assert r['comparison']['valid'] and r['comparison']['gates']['complete_response_parity']"])
        (self.notebook / "contract.toml").write_text(
            'version = 1\n[project]\nname = "rust-pi-resources"\n'
            'objective = "Compare Rust resource cost under the unchanged paired evaluator"\n'
            'program = ".autoresearch/program.md"\n[scope]\nmutable_paths = []\n'
            'protected_paths = ["scripts/bench-paired-pi.py", "scripts/bench-rust-pi.py", "source-manifests/baseline.json", "source-manifests/candidate.json"]\n'
            f'[commands]\nsmoke = {json.dumps(self.smoke_command)}\n'
            f'evaluate = {json.dumps(shlex.join(command))}\n'
            f'verify = {json.dumps(self.verify_command)}\n'
            'evaluate_requires_confirmation = false\n[campaign]\ndefault_max_experiments = 6\n'
            'default_max_minutes = 20\nrepetitions = 3\n'
            '[acceptance]\npolicy = "primary-with-gates"\n'
            'primary_metric = "cpu_seconds_per_300_requests_median"\n'
            'minimum_relative_cpu_reduction = 0.05\nmaximum_rss_increase_KiB = 1024\n'
            'maximum_workload_p50_regression_fraction = 0.10\n'
            '[integrity]\nhash_paths = ["scripts/bench-paired-pi.py", "scripts/bench-rust-pi.py", "source-manifests/baseline.json", "source-manifests/candidate.json"]\n'
            '[[metrics]]\nname = "cpu_seconds_per_300_requests_median"\ndirection = "minimize"\nrole = "primary"\n'
            '[[metrics]]\nname = "rss_median_KiB"\ndirection = "minimize"\nrole = "gate"\ntolerance = 1024\n'
            '[[metrics]]\nname = "workload_p50_median_ms"\ndirection = "minimize"\nrole = "gate"\nrelative_tolerance = 0.10\n'
            '[[metrics]]\nname = "complete_response_parity"\ndirection = "maximize"\nrole = "gate"\nthreshold = 1\n')
        (self.notebook / "program.md").write_text(
            "# Fixed Rust Pi comparison\n\n"
            "One unchanged evaluator, synthetic corpus, seed 20261015, 100 samples and "
            "10 warmups per workload; three alternating baseline/candidate rounds. "
            "Server 1 CPU/768 MiB, DB 1 CPU/512 MiB, mock 0.25 CPU/128 MiB; no container swap. "
            "Only the pinned Rust image changes. Generation, external model calls, production "
            "mutations and deployment are outside this run. The inherited coordinator checks "
            "a 1100-second deadline and 512-MiB host headroom at progress points. "
            "The wrapper interrupts core work after 1200 seconds and then cleans up owned resources. "
            "Any error, OOM, response-hash disagreement, mock-count disagreement or protected "
            "file change stops the comparison. Missing runs are not successes. "
            "Keep only if median server CPU per 300 measured requests falls at least 5%, "
            "all complete response hashes match, all requests succeed, cleanup succeeds, "
            "median final server RSS grows no more than 1024 KiB and each workload's median "
            "round p50 latency regresses no more than 10%. Report all three pairs and raw "
            "distributions. Invalid/incomplete comparisons are inconclusive/defer. "
            "A keep decision does not deploy or authorize another attempt.\n")
        self.save()
        self.checkpoint("prepared; no Rust evaluation launched")

    def integrity(self):
        return {path: {"expected": expected,
                       "actual": digest(self.root / path) if (self.root / path).is_file() else None}
                for path, expected in self.protected.items()}

    def check_integrity(self):
        require(all(item["expected"] == item["actual"] for item in self.integrity().values()),
                "protected evaluator/coordinator changed during the comparison")

    def checkpoint(self, state):
        temporary = self.notebook / "CHECKPOINT.tmp"
        temporary.write_text(
            f"# Rust Pi comparison\n\nComparison: {self.comparison_group}\n\n"
            f"State: {state}\n\nUpdated: {now()}\n\n"
            f"Active run: {self.active['run_id'] if self.active else 'none'}\n\n"
            "Evidence: runs/*/{manifest.json,results.jsonl}; ../report.json. "
            "Six executions maximum, no automatic retries. Reconcile owned containers and "
            "terminal evidence before any new comparison; use a new root for a retry.\n")
        temporary.replace(self.notebook / "CHECKPOINT.md")

    def event(self, run, event_type, **payload):
        record = {"event_id": uuid.uuid4().hex, "run_id": run["run_id"],
                  "recorded_at": now(), "type": event_type, **payload}
        with (run["directory"] / "results.jsonl").open("a") as stream:
            stream.write(base.encode(record) + "\n")
            stream.flush()
            os.fsync(stream.fileno())

    def preflight(self):
        self.check_integrity()
        for kind, build in self.builds.items():
            info = json.loads(self.docker("image", "inspect", build["image_id"]))[0]
            require(info["Id"] == build["image_id"] and info["Os"] == "linux"
                    and info["Architecture"] == "arm64", f"{kind} image must be local Linux ARM64")
            labels = info["Config"].get("Labels") or {}
            revision = labels.get("org.opencontainers.image.revision")
            require(revision is None or revision == build["source_commit"],
                    f"{kind} image revision label disagrees with supplied commit")
            build["revision_label"] = revision
        # DB and mock also use a pre-existing pinned local image, never a pull.
        self.docker("image", "inspect", base.PYTHON_IMAGE)
        self.save()

    def manifest(self, run, image, argv, memory, cpus, network, mounts):
        self.check_integrity()
        config = {"argv": argv, "image_id": image, "memory_MiB": memory, "cpus": cpus,
                  "network": network, "mounts": list(mounts), "limits": self.report["limits"],
                  "design": self.report["design"], "seed": 20261015,
                  "preprocessing": "unchanged base.canonical and synthetic fixture/vector functions"}
        config_path = run["directory"] / "config.json"
        write_json(config_path, config)
        baseline = self.runs[run["trial"], "baseline"]["run_id"]
        build = self.builds[run["kind"]]
        write_json(run["directory"] / "manifest.json", {
            "schema": "autoresearch-notebook/v1", "run_id": run["run_id"],
            "created_at": now(), "record_origin": "prospective", "kind": run["kind"],
            "baseline_id": baseline if run["kind"] == "candidate" else None,
            "comparison_group": self.comparison_group, "retry_of": None,
            "hypothesis": "Candidate reduces Rust retrieval CPU/allocation cost while preserving complete responses"
                          if run["kind"] == "candidate" else "Measure the unchanged Rust baseline",
            "expected_effect": "Lower request CPU/RSS and no response, error, or provider-call regression",
            "falsifier": "Response/hash/count mismatch, OOM/error, or no reproducible resource improvement",
            "code": {"repository": "https://github.com/andyjusa/graphiti-rs",
                     "revision": build["source_commit"], "revision_provenance": "operator-supplied CLI; image label checked when present",
                     "branch": None, "dirty_status": None, "executed_source_identity": image,
                     "immutable_artifact": image, "revision_label": build["revision_label"],
                     "source_build_manifest": build["source_build_manifest"],
                     "expected_binary_sha256": build["expected_binary_sha256"]},
            "config": {"path": str(config_path.relative_to(self.root)), "sha256": digest(config_path)},
            "data": self.report["corpus"],
            "environment": {"host": platform.node(), "architecture": platform.machine(),
                            "kernel": platform.release(), "image_id": image,
                            "support_image_id": base.PYTHON_IMAGE, "accelerator": "none"},
            "contract": {"path": ".autoresearch/contract.toml",
                         "sha256": digest(self.notebook / "contract.toml"),
                         "program": ".autoresearch/program.md",
                         "program_sha256": digest(self.notebook / "program.md"),
                         "protected_path_sha256": self.protected, "acceptance_policy": "primary-with-gates"},
            "execution": {"working_directory": str(self.root), "coordinator_command": self.command,
                          "evaluate": f"paired_pi.Experiment.run({run['kind']!r}, {run['trial']}, shared_response_hashes)",
                          "smoke_command": self.smoke_command, "verify_command": self.verify_command,
                          "readiness": "inherited MCP initialize and tools/list (ten tools)",
                          "server_argv": argv},
            "budget": {"authorization_reference": "explicit bounded Rust baseline/candidate Pi comparison",
                       "executions_maximum": 6, "planned_repetitions": 3,
                       "run_allocation": {"round": run["trial"] + 1, "measured_requests": 300, "warmups": 30},
                       "inherited_progress_deadline_seconds": 1100,
                       "core_work_maximum_seconds": 1200, "stopping_rule": "first failure; no retries"},
            "unknown_reasons": {"code.branch": "built image has no checkout",
                                "code.dirty_status": "build working-tree state is not observable from the immutable image"},
        })

    def container(self, suffix, image, argv, memory, cpus, network=None, mounts=()):
        run = self.active
        if run:
            self.manifest(run, image, argv, memory, cpus, network, mounts)
        name = super().container(suffix, image, argv, memory, cpus, network, mounts)
        if run:
            info = self.inspect(name)
            run["container_name"] = name
            self.event(run, "launch", phase="evaluation", started_at=info["State"]["StartedAt"],
                       host=platform.node(), container_id=info["Id"], container_name=name,
                       process_id=info["State"]["Pid"], log_paths=[name + ".log"],
                       allocated_resources={"cpus": cpus, "memory_MiB": memory})
            self.checkpoint("container launched; readiness/evaluation in progress")
        return name

    def start_server(self, kind, trial):
        require(kind in self.builds and self.active is not None, "unexpected Rust run kind")
        base.RUST_IMAGE = self.builds[kind]["image_id"]
        ready = super().start_server("rust", f"{kind}-{trial}")
        binary_hash = self.report.pop("rust_binary_sha256")
        self.active["binary_sha256"] = binary_hash
        self.event(self.active, "observation", state="running", observation_source="inherited MCP readiness",
                   observation_time=now(), container_name=ready[0], process_id=ready[1],
                   binary_sha256=binary_hash, image_id=base.RUST_IMAGE)
        require(binary_hash == self.builds[kind]["expected_binary_sha256"],
                f"{kind} running binary differs from its source build manifest")
        previous = self.builds[kind].get("binary_sha256")
        require(previous is None or previous == binary_hash, "binary changed between same-image runs")
        self.builds[kind]["binary_sha256"] = binary_hash
        return ready

    def run(self, kind, trial, reference):
        self.active = self.runs[trial, kind]
        began = base.time.monotonic()
        status, error, failure_class = "completed", None, None
        try:
            super().run(kind, trial, reference)
            self.check_integrity()
        except BaseException as caught:
            status = "cancelled" if isinstance(caught, (KeyboardInterrupt, SystemExit)) else "failed"
            error = str(caught)
            failure_class = type(caught).__name__
            raise
        finally:
            record = next((r for r in reversed(self.report["runs"])
                           if r["kind"] == kind and r["round"] == trial + 1), None)
            if record is not None:
                record.update(run_id=self.active["run_id"], image_id=self.builds[kind]["image_id"],
                              source_commit=self.builds[kind]["source_commit"],
                              binary_sha256=self.active.get("binary_sha256"))
            self.event(self.active, "result", phase="evaluation", execution_status=status,
                       ended_at=now(), exit_code=None, failure_class=failure_class,
                       failure_reason=error, completed_repetitions=int(status == "completed"),
                       raw_metric_artifacts=["report.json"], metric_summary=record,
                       uncertainty="individual run; retain raw samples and all three pairs, no confidence claim",
                       actual_budget_seconds=base.time.monotonic() - began,
                       smoke_outcome="passed" if self.active.get("binary_sha256") else "not_confirmed",
                       verification_outcome="passed" if status == "completed" else "incomplete",
                       protected_hash_check=self.integrity(),
                       unknown_reasons={"exit_code": "in-process evaluator; no separate evaluation process exit code"})
            self.active = None
            self.checkpoint(f"{kind} round {trial + 1}: {status}")
            self.save()

    def decisions(self):
        summaries = {}
        workloads = ("episode_read", "hybrid_fact_search", "semantic_node_search")
        complete = self.report["status"] == "completed" and len(self.report["runs"]) == 6
        for kind in self.builds:
            runs = [run for run in self.report["runs"] if run["kind"] == kind]
            complete = complete and len(runs) == 3 and all(
                "final" in run and not run.get("container_oom_killed", True)
                and set(run["workloads"]) == set(workloads)
                and all(w["successes"] == 100 and w["errors"] == 0 for w in run["workloads"].values())
                for run in runs)
            if len(runs) == 3 and all("final" in run and set(run["workloads"]) == set(workloads) for run in runs):
                summaries[kind] = {
                    "cpu_seconds_per_300_requests_median": base.statistics.median(
                        sum(w["server_cpu_seconds"] for w in run["workloads"].values()) for run in runs),
                    "rss_median_KiB": base.statistics.median(run["final"]["VmRSS_KiB"] for run in runs),
                    "workload_p50_median_ms": {workload: base.statistics.median(
                        run["workloads"][workload]["p50_ms"] for run in runs) for workload in workloads}}
        parity = complete and len(self.report.get("response_parity", {})) == 3 and all(
            len({run["workloads"][workload]["response_sha256"] for run in self.report["runs"]}) == 1
            for workload in workloads)
        valid = bool(complete and parity and not self.report.get("cleanup_errors")
                     and all(item["expected"] == item["actual"] for item in self.integrity().values()))
        gates = {"all_requests_succeeded_and_no_oom": bool(complete), "complete_response_parity": bool(parity),
                 "cleanup_succeeded": not self.report.get("cleanup_errors"),
                 "protected_paths_unchanged": all(item["expected"] == item["actual"] for item in self.integrity().values())}
        deltas = None
        decision, conclusion = "defer", "inconclusive"
        if valid:
            baseline, candidate = summaries["baseline"], summaries["candidate"]
            cpu = baseline["cpu_seconds_per_300_requests_median"]
            if cpu <= 0 or any(value <= 0 for value in baseline["workload_p50_median_ms"].values()):
                valid = False
            else:
                deltas = {"cpu_reduction_fraction": 1 - candidate["cpu_seconds_per_300_requests_median"] / cpu,
                          "rss_increase_KiB": candidate["rss_median_KiB"] - baseline["rss_median_KiB"],
                          "workload_p50_regression_fraction": {workload:
                              candidate["workload_p50_median_ms"][workload] / baseline["workload_p50_median_ms"][workload] - 1
                              for workload in workloads}}
                gates["rss_increase_at_most_1024_KiB"] = deltas["rss_increase_KiB"] <= 1024
                gates["each_workload_p50_regression_at_most_10_percent"] = all(
                    candidate["workload_p50_median_ms"][workload] <= baseline["workload_p50_median_ms"][workload] * 1.10
                    for workload in workloads)
                improved = candidate["cpu_seconds_per_300_requests_median"] <= cpu * 0.95 and all(gates.values())
                decision, conclusion = ("keep", "passed") if improved else ("reject", "acceptance_not_met")
        self.report["comparison"] = {"valid": valid, "conclusion": conclusion, "decision": decision,
                                     "summaries": summaries, "metric_deltas": deltas, "gates": gates,
                                     "acceptance": {"minimum_cpu_reduction_fraction": 0.05,
                                                    "maximum_rss_increase_KiB": 1024,
                                                    "maximum_workload_p50_regression_fraction": 0.10}}
        for trial in range(3):
            run = self.runs[trial, "candidate"]
            if not (run["directory"] / "manifest.json").exists():
                continue
            self.event(run, "decision", baseline_id=self.runs[trial, "baseline"]["run_id"],
                       comparability="matched" if valid else "inconclusive",
                       mismatches=[] if valid else ["fixed comparison/integrity gates incomplete; inspect report.json"],
                       gates=gates, acceptance_policy="primary-with-gates",
                       decision=decision, rationale=conclusion,
                       metric_deltas=deltas, known_limitations=["synthetic retrieval only; no ingestion or real inference",
                                                             "shared warm DB; alternating order does not remove all host noise"],
                       next_action="Review report.json paired runs without changing evaluator or budgets",
                       remaining_authorized_executions=0)


def main():
    require(platform.system() == "Linux" and platform.machine() == "aarch64",
            "Pi Linux ARM64 only; never benchmark on Mac")
    require(not sys.flags.optimize, "Python optimization disables protected evaluator assertions")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    for kind in ("baseline", "candidate"):
        parser.add_argument(f"--{kind}-image", required=True)
        parser.add_argument(f"--{kind}-commit", required=True)
    args = parser.parse_args()
    require(not args.root.is_symlink(), "owned benchmark root must not be a symlink")
    root = args.root.resolve(strict=True)
    require(root.parent == Path("/home/pi4-3") and re.fullmatch(
        r"graphiti-rust-bench-20261016-[A-Za-z0-9][A-Za-z0-9_-]*", root.name), "invalid owned root")
    require(root.is_dir() and root.stat().st_uid == os.getuid()
            and stat.S_IMODE(root.stat().st_mode) == 0o700, "root must be owned and mode 0700")
    marker = root / "OWNED"
    require(not marker.is_symlink() and marker.is_file() and marker.read_text().strip() == MARKER,
            "owned root marker mismatch")
    require(HERE == root / "scripts", "execute only the coordinator staged inside the owned root")
    builds = {}
    for kind in ("baseline", "candidate"):
        image, commit = getattr(args, kind + "_image"), getattr(args, kind + "_commit")
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", image), f"{kind} image must be a sha256 image ID")
        require(re.fullmatch(r"[0-9a-f]{40}", commit), f"{kind} commit must be a full SHA-1")
        builds[kind] = {"image_id": image, "source_commit": commit}
    experiment = Experiment(root, builds, [sys.executable, *sys.argv])

    def interrupted(number, _):
        raise SystemExit(f"interrupted by signal {number}")

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    signal.alarm(1200)
    try:
        experiment.preflight()
        experiment.prepare()
        reference = {}
        for trial, order in enumerate(experiment.report["design"]["orders"]):
            for kind in order:
                experiment.run(kind, trial, reference)
        experiment.report["response_parity"] = reference
        experiment.report["status"] = "completed"
        experiment.progress("six Rust runs passed with identical complete canonical responses")
    except BaseException as error:
        experiment.report["status"] = "failed"
        experiment.report["error"] = str(error)
        raise
    finally:
        signal.alarm(0)
        experiment.cleanup()
        experiment.decisions()
        experiment.save()
        experiment.checkpoint(experiment.report["status"] + "; inspect cleanup_errors and evidence before retrying")


if __name__ == "__main__":
    main()
