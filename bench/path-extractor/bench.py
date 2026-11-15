"""Benchmark: the deterministic extractor against a small local model on events.toml.

    python -I bench.py --out results.json
    python -I bench.py --out results.json --server <llama-server.exe> --model <model.gguf> [--threads 4]

Both extractors give raw operations; extract.resolve turns them into candidates. Scored on
(op, dst) after resolution: correct, invented (not in gold), missed (gold not found)."""
import argparse
import json
import statistics
import subprocess
import sys
import time
import tomllib
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from extract import extract, resolve  # noqa: E402

SYSTEM = """You read one shell command and list, in order, the filesystem operations it performs.
op is one of:
create: makes a new directory or project at dst
copy: copies or extracts src into dst
move: moves src to dst
clone: git clone of src into dst (dst is null when the command names none)
worktree: git worktree add at dst
write: writes the file dst, or a tool writes its output into the directory dst
cd: changes the working directory to dst
Copy every path exactly as it is written in the command. src is null except for copy, move and clone.
A command that only reads, lists, searches, prints or deletes gives {"ops": []}."""

# Teaching examples: other paths and commands than events.toml.
SHOTS = [
    ("bash", r"E:\work\app", "mkdir -p /e/tmp/demo && cd /e/tmp/demo",
     [("create", None, "/e/tmp/demo"), ("cd", None, "/e/tmp/demo")]),
    ("powershell", r"E:\work\app", r"Copy-Item -Recurse C:\src\app C:\copies\app-old",
     [("copy", r"C:\src\app", r"C:\copies\app-old")]),
    ("bash", r"E:\work\app", "git clone https://example.com/team/tool.git",
     [("clone", "https://example.com/team/tool.git", None)]),
    ("bash", r"E:\work\app", "ls -la build && git status", []),
    ("bash", r"E:\work\app", "npm run lint > lint.txt 2>&1", [("write", None, "lint.txt")]),
    ("powershell", r"E:\work\app", r"Remove-Item -Recurse C:\work\old", []),
    ("powershell", r"E:\work\app", r"git worktree add C:\wt\app-hotfix -b hotfix",
     [("worktree", None, r"C:\wt\app-hotfix")]),
    ("cmd", r"E:\work\app", r"cd /d E:\data && md batch-7", [("cd", None, r"E:\data"), ("create", None, "batch-7")]),
]

NULLABLE = {"anyOf": [{"type": "string", "maxLength": 160}, {"type": "null"}]}
SCHEMA = {
    "type": "object",
    "properties": {"ops": {"type": "array", "maxItems": 8, "items": {
        "type": "object",
        "properties": {"op": {"enum": ["create", "copy", "move", "clone", "worktree", "write", "cd"]},
                       "src": NULLABLE, "dst": NULLABLE},
        "required": ["op", "src", "dst"], "additionalProperties": False}}},
    "required": ["ops"], "additionalProperties": False,
}


def user_text(shell, cwd, cmd):
    # Forward slashes: a 135M model loops on JSON-escaped backslashes; the resolver reads both.
    return f"shell: {shell}\ncwd: {cwd}\ncommand: {cmd}".replace("\\", "/")


def messages(ev):
    m = [{"role": "system", "content": SYSTEM}]
    for shell, cwd, cmd, ops in SHOTS:
        m.append({"role": "user", "content": user_text(shell, cwd, cmd)})
        m.append({"role": "assistant", "content": json.dumps(
            {"ops": [{"op": o, "src": s, "dst": d} for o, s, d in ops]}).replace("\\\\", "/")})
    m.append({"role": "user", "content": user_text(ev["shell"], ev["cwd"], ev["cmd"])})
    return m


def load(path):
    data = tomllib.loads(Path(path).read_text(encoding="utf-8"))
    d = data["defaults"]
    env = {"home": d["home"], "temp": d["temp"]}
    events = [{"cwd": d["cwd"], "exit": 0, **e, "cmd": e["cmd"].strip()} for e in data["event"]]
    return events, env


def key(c):
    return (c["op"], c["dst"].lower())


def spans_ok(o, cmd):
    """Guard: every path the model gives must be written in the command (slashes and case aside)."""
    text = cmd.replace("\\", "/").lower()
    return all(p is None or p.strip() in (".", "") or p.replace("\\", "/").lower().strip() in text
               for p in (o.get("src"), o.get("dst")))


def score(events, field):
    rows = {}
    for ev in events:
        got = {key(c): c for c in ev[field]}
        gold = {key(c): c for c in ev["gold_c"]}
        r = rows.setdefault(ev["cat"], {"events": 0, "exact": 0, "correct": 0, "invented": 0, "missed": 0,
                                        "src_ok": 0, "src_n": 0})
        r["events"] += 1
        r["exact"] += got.keys() == gold.keys()
        r["correct"] += len(got.keys() & gold.keys())
        r["invented"] += len(got.keys() - gold.keys())
        r["missed"] += len(gold.keys() - got.keys())
        for k in got.keys() & gold.keys():
            if gold[k].get("src"):
                r["src_n"] += 1
                r["src_ok"] += str(got[k].get("src") or "").lower() == gold[k]["src"].lower()
    total = {k: sum(r[k] for r in rows.values()) for k in next(iter(rows.values()))}
    return total, rows


def ram(pid):
    out = subprocess.run(["powershell", "-NoProfile", "-Command",
                          f"$p = Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PeakWorkingSet64) $($p.PrivateMemorySize64)\""],
                         capture_output=True, text=True, timeout=60).stdout.split()
    return dict(zip(("working_set", "peak_working_set", "private"), map(int, out)))


def post(port, body):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/chat/completions", json.dumps(body).encode(),
                                 {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.load(r)


def run_model(events, env, args):
    log = open(Path(args.out).with_suffix(".server.log"), "w", encoding="utf-8")
    t0 = time.perf_counter()
    srv = subprocess.Popen([args.server, "-m", args.model, "-t", str(args.threads), "-c", "4096", "-np", "1",
                            "--port", str(args.port), "--host", "127.0.0.1"], stdout=log, stderr=subprocess.STDOUT)
    try:
        while True:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{args.port}/health", timeout=2).read(); break
            except Exception:
                if srv.poll() is not None or time.perf_counter() - t0 > 120:
                    raise SystemExit(f"llama-server did not start, see {log.name}")
                time.sleep(0.2)
        info = {"load_s": round(time.perf_counter() - t0, 2), "ram_loaded": ram(srv.pid)}
        for ev in events:
            body = {"messages": messages(ev), "temperature": 0, "max_tokens": 256, "cache_prompt": True,
                    "stop": ["<|im_end|>"],
                    "response_format": {"type": "json_schema", "json_schema": {"name": "ops", "schema": SCHEMA}}}
            t = time.perf_counter()
            r = post(args.port, body)
            ev["model_ms"] = (time.perf_counter() - t) * 1000
            ev["model_timings"] = r.get("timings", {})
            text = r["choices"][0]["message"]["content"]
            try:  # the first JSON object; anything after it is ignored
                ops = json.JSONDecoder().raw_decode(text.strip())[0]["ops"]
            except (ValueError, KeyError, TypeError):
                ops, ev["model_bad_json"] = [], text
            ev["model_raw"] = ops
            ev["model_c"], ev["model_unresolved"] = resolve(ops, ev["cwd"], env)
            guarded = [o for o in ops if spans_ok(o, ev["cmd"])]
            ev["guard_c"], _ = resolve(guarded, ev["cwd"], env)
        info["ram_end"] = ram(srv.pid)
        return info
    finally:
        srv.terminate()
        srv.wait(timeout=30)
        log.close()


def table(name, total):
    n = total["correct"] + total["missed"]
    p = total["correct"] / max(1, total["correct"] + total["invented"])
    print(f"{name:<22} exact {total['exact']:>3}/{total['events']:<3} correct {total['correct']:>3}/{n:<3} "
          f"invented {total['invented']:>3}  missed {total['missed']:>3}  precision {p:.2f}  "
          f"src {total['src_ok']}/{total['src_n']}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--events", default=str(Path(__file__).with_name("events.toml")))
    ap.add_argument("--out", required=True)
    ap.add_argument("--server")
    ap.add_argument("--model")
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--port", type=int, default=8765)
    args = ap.parse_args()
    events, env = load(args.events)

    t = time.perf_counter()
    for ev in events:
        ops, ev["det_flags"] = extract(ev["cmd"], ev["shell"])
        ev["det_raw"] = ops
        ev["det_c"], ev["det_unresolved"] = resolve(ops, ev["cwd"], env)
        ev["gold_c"] = [{"op": g["op"], "src": g.get("src"), "dst": g["dst"]} for g in ev["gold"]]
    det_ms = (time.perf_counter() - t) * 1000 / len(events)

    fields = [("deterministic", "det_c")]
    info = {}
    if args.model:
        info = run_model(events, env, args)
        for ev in events:  # hybrid: the model only where the deterministic extractor says it is unsure
            unsure = ev["det_flags"] or ev["det_unresolved"]
            extra = [c for c in ev["guard_c"] if unsure and key(c) not in {key(d) for d in ev["det_c"]}]
            ev["hybrid_c"] = ev["det_c"] + extra
        fields += [("model", "model_c"), ("model+span guard", "guard_c"), ("hybrid", "hybrid_c")]

    summary = {}
    for name, field in fields:
        total, rows = score(events, field)
        summary[name] = {"total": total, "by_category": rows}
        table(name, total)
    for name, field in fields:
        print(f"\n{name}: events not exact")
        for ev in events:
            if {key(c) for c in ev[field]} != {key(c) for c in ev["gold_c"]}:
                print(f"  {ev['id']:<4} {ev['cat']:<9} got {[(c['op'], c['dst']) for c in ev[field]]}")
    print(f"\ndeterministic: {det_ms:.3f} ms/event (Python)")
    if args.model:
        ms = [ev["model_ms"] for ev in events[1:]]
        info |= {"model_ms_first": round(events[0]["model_ms"]), "model_ms_median": round(statistics.median(ms)),
                 "model_ms_p95": round(sorted(ms)[int(len(ms) * 0.95) - 1]), "model_ms_max": round(max(ms)),
                 "bad_json": sum("model_bad_json" in ev for ev in events)}
        print("model:", json.dumps(info))
    Path(args.out).write_text(json.dumps({"summary": summary, "info": info, "det_ms_per_event": det_ms,
                                          "events": events}, indent=1, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    main()
