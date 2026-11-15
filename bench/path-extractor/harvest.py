"""Collect every tool call of every readable session, of every harness store given, through Mukade.

    python harvest.py --mukade <mukade.exe> --stores stores.tsv --out calls.jsonl [--workers 6]

stores.tsv: `<harness>\t<store root>` per line. Only `mukade list` and `mukade dump --stream` run:
both read the stores without writing them. Each output line is one call: harness, session, entry,
tool name (source and canonical), input (long strings cut), the cwd of the call, and the outcome.
Result texts are kept only as a short head, enough to read an exit code."""
import argparse
import json
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed

MAX_STR = 4000
EXIT = re.compile(r"(?:exit(?:ed with)? code|exit status)[:\s]+(-?\d+)", re.I)


def cut(v):
    if isinstance(v, str):
        return v if len(v) <= MAX_STR else v[:MAX_STR] + "…"
    if isinstance(v, dict):
        return {k: cut(x) for k, x in v.items()}
    if isinstance(v, list):
        return [cut(x) for x in v]
    return v


def mukade(exe, extra, *args, timeout=600):
    return subprocess.run([exe, *args, *extra], capture_output=True, timeout=timeout)


def sessions(exe, harness, root, extra):
    p = mukade(exe, extra, "list", harness, "--root", root, "--children")
    rows = [json.loads(l) for l in p.stdout.decode("utf-8", "replace").split("\n") if l.strip()]
    return rows, p.returncode, p.stderr.decode("utf-8", "replace")[:300]


def calls_of(exe, harness, root, row, extra):
    try:
        p = mukade(exe, extra, "dump", harness, row["id"], "--root", root, "--stream")
    except subprocess.TimeoutExpired:
        return [], "timeout"
    if p.returncode != 0:
        return [], p.stderr.decode("utf-8", "replace").strip()[:200]
    pending, out = {}, []
    for line in p.stdout.decode("utf-8", "replace").split("\n"):  # not splitlines: U+2028 is inside strings
        if not line.strip():
            continue
        try:
            d = json.loads(line)
        except ValueError:
            continue
        kind = d.get("kind")
        if not isinstance(kind, dict):
            continue
        raw = d.get("raw") or [{}]
        raw0 = raw[0] if isinstance(raw, list) and raw and isinstance(raw[0], dict) else {}
        step = kind.get("step")
        if step:
            for c in step.get("calls") or []:
                inp = c.get("input") if isinstance(c.get("input"), dict) else {"_": c.get("input")}
                cwd = next((inp[k] for k in ("workdir", "cwd", "working_directory", "directory")
                            if isinstance(inp.get(k), str)), None) or raw0.get("cwd") or row.get("cwd")
                rec = {"harness": harness, "store": root, "session": row["id"], "parent": row.get("parent"),
                       "entry": d.get("id"), "call": c.get("id"), "tool": c.get("source_name"),
                       "canonical": c.get("canonical"), "input": cut(inp), "cwd": cwd,
                       "time": raw0.get("timestamp") or raw0.get("time"), "error": None, "exit": None,
                       "result_head": None}
                out.append(rec)
                if c.get("id"):
                    pending[c["id"]] = rec
        res = kind.get("results")
        if res:
            for r in res.get("results") or []:
                rec = pending.pop(r.get("call_id"), None)
                if rec is None:
                    continue
                content = r.get("content")
                text = content.get("text") if isinstance(content, dict) else content if isinstance(content, str) else ""
                text = text or ""
                m = EXIT.search(text[:2000]) or EXIT.search(text[-500:])
                rec["error"], rec["exit"] = r.get("is_error"), int(m[1]) if m else None
                rec["result_head"] = text[:300]
    return out, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mukade", required=True)
    ap.add_argument("--stores", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--opencode-exe")
    args = ap.parse_args()
    stores = [l.rstrip("\n").split("\t") for l in open(args.stores, encoding="utf-8") if l.strip()]
    report = []
    with open(args.out, "w", encoding="utf-8") as out:
        for harness, root in stores:
            extra = ["--opencode-exe", args.opencode_exe] if harness == "opencode" and args.opencode_exe else []
            rows, rc, err = sessions(args.mukade, harness, root, extra)
            todo = [r for r in rows if not r.get("unreadable")]
            stat = {"harness": harness, "store": root, "listed": len(rows), "readable": len(todo), "read": 0,
                    "failed": 0, "calls": 0, "failures": {}, "list_rc": rc, "list_err": err}
            with ThreadPoolExecutor(args.workers) as pool:
                futs = [pool.submit(calls_of, args.mukade, harness, root, r, extra) for r in todo]
                for i, f in enumerate(as_completed(futs), 1):
                    recs, fail = f.result()
                    if fail:
                        stat["failed"] += 1
                        key = re.sub(r"[0-9a-f-]{8,}|ses_\w+|local-\w+", "<id>", fail)[:120]
                        stat["failures"][key] = stat["failures"].get(key, 0) + 1
                    else:
                        stat["read"] += 1
                    stat["calls"] += len(recs)
                    for r in recs:
                        out.write(json.dumps(r, ensure_ascii=False) + "\n")
                    if i % 100 == 0:
                        print(f"  {harness} {i}/{len(todo)}", file=sys.stderr, flush=True)
            report.append(stat)
            print(json.dumps(stat, ensure_ascii=False), file=sys.stderr, flush=True)
    with open(args.out + ".report.json", "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
