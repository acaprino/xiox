"""Folder evidence per session: what the session shows about each folder, from every event in it.

A folder can be missed by the parser of one command (a script, a variable) and still be shown by the
session elsewhere: a structured Write or Read, another command, a cd. The ledger keeps, per folder, the
strongest evidence and where it comes from. A failed operation stays visible but never counts as a
creation.

    python evidence.py --calls calls.jsonl --sample heldout.json --labels labels.json
        recall of the labelled folders outside the cwd: from the command alone, and from the session
"""
import argparse
import json
import ntpath
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from enum import IntEnum
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from extract import Unresolved, extract, resolve, resolve_path  # noqa: E402
from real import ENV, shell_of  # noqa: E402


class Strength(IntEnum):
    USED = 1      # read, searched, entered with cd, or the cwd of a call
    WRITTEN = 2   # a file in it was written
    CREATED = 3   # made, copied, moved, cloned or added as a worktree


@dataclass(frozen=True)
class Ref:
    session: str
    entry: str | None
    call: str | None
    source: str   # "structured:<tool>", "shell:<op>", "cd", "cwd"


@dataclass(frozen=True)
class Evidence:
    folder: str
    strength: Strength
    ref: Ref
    failed: bool = False


@dataclass
class FolderEvidence:
    path: str
    strength: Strength | None = None          # the strongest evidence that did not fail
    refs: list[Ref] = field(default_factory=list)
    failed: list[Ref] = field(default_factory=list)


class Ledger:
    """Evidence of one session, keyed by folder (case-insensitive, as on Windows)."""

    def __init__(self):
        self._folders: dict[str, FolderEvidence] = {}

    def add(self, ev: Evidence):
        entry = self._folders.setdefault(ev.folder.lower(), FolderEvidence(ev.folder))
        if ev.failed:
            entry.failed.append(ev.ref)
            return
        entry.refs.append(ev.ref)
        entry.strength = max(entry.strength or ev.strength, ev.strength)

    def get(self, folder: str) -> FolderEvidence | None:
        return self._folders.get(folder.lower())

    def __iter__(self):
        return iter(self._folders.values())


# ---- from candidates and fields to folders

DIRECTORY_OPS = {"create", "clone", "worktree"}
STRENGTH_OF = {"create": Strength.CREATED, "clone": Strength.CREATED, "worktree": Strength.CREATED,
               "copy": Strength.CREATED, "move": Strength.CREATED, "write": Strength.WRITTEN}
STRUCTURED = {"write": Strength.WRITTEN, "edit": Strength.WRITTEN, "notebook_edit": Strength.WRITTEN,
              "read": Strength.USED, "glob": Strength.USED, "grep": Strength.USED}
PATH_FIELDS = ("file_path", "notebook_path", "path")


def folder_of(op: str, path: str) -> str:
    """The folder a candidate speaks about. shortcut: a last segment with an extension is taken for a
    file, so `--out D:\\runs\\v1.2` reads as a file; upgrade when the result text shows the type."""
    if op in DIRECTORY_OPS:
        return path
    name = ntpath.basename(path)
    return ntpath.dirname(path) if "." in name.strip(".") else path


def failed(call: dict) -> bool:
    return bool(call.get("error")) or call.get("exit") not in (None, 0)


def evidence_of(call: dict):
    """Every piece of folder evidence one tool call gives."""
    cwd = (call.get("cwd") or "").removeprefix("\\\\?\\").replace("/", "\\") or None
    ref = lambda source: Ref(call["session"], call.get("entry"), call.get("call"), source)  # noqa: E731
    bad = failed(call)
    if cwd:
        yield Evidence(cwd, Strength.USED, ref("cwd"))
    canonical = call.get("canonical") if isinstance(call.get("canonical"), str) else None
    if canonical in STRUCTURED:
        for k in PATH_FIELDS:
            value = call["input"].get(k)
            if not isinstance(value, str) or not value:
                continue
            try:
                path = resolve_path(value, cwd, ENV)
            except Unresolved:
                continue
            is_dir = canonical in ("glob", "grep")
            yield Evidence(path if is_dir else ntpath.dirname(path), STRUCTURED[canonical],
                           ref(f"structured:{call['tool']}"), bad)
        return
    shell = shell_of(call)
    if shell and cwd:
        raw, _ = extract(shell[1], shell[0])
        visits = []  # each cd resolved where it ran, not against the cwd of the call
        for c in resolve(raw, cwd, ENV, visits)[0]:
            yield Evidence(folder_of(c["op"], c["dst"]), STRENGTH_OF[c["op"]], ref(f"shell:{c['op']}"), bad)
        for folder in visits:
            yield Evidence(folder, Strength.USED, ref("cd"))


def ledgers(calls_path: str, sessions: set[str] | None = None) -> dict[str, Ledger]:
    out: dict[str, Ledger] = defaultdict(Ledger)
    with open(calls_path, encoding="utf-8") as f:
        for line in f:
            call = json.loads(line)
            if sessions is None or call["session"] in sessions:
                for ev in evidence_of(call):
                    out[call["session"]].add(ev)
    return out


# ---- evaluation against labels

def outside(path: str, cwd: str) -> bool:
    p, c = path.lower().rstrip("\\"), cwd.lower().rstrip("\\")
    return not (p == c or p.startswith(c + "\\"))


def evaluate(args):
    sample = json.loads(Path(args.sample).read_text(encoding="utf-8"))["events"]
    labels = json.loads(Path(args.labels).read_text(encoding="utf-8"))
    books = ledgers(args.calls, {ev["session"] for ev in sample})
    tally = defaultdict(int)
    for ev in sample:
        own = {folder_of(c["op"], c["dst"]).lower()
               for c in resolve(extract(ev["cmd"], ev["shell"])[0], ev["cwd"], ENV)[0]}
        for op, path in labels.get(ev["id"], []):
            if not outside(path, ev["cwd"]):
                continue
            folder = folder_of(op, path)
            found = books[ev["session"]].get(folder)
            tally["gold"] += 1
            tally["command"] += folder.lower() in own
            tally["session"] += found is not None and found.strength is not None
            tally["session_written"] += found is not None and (found.strength or 0) >= Strength.WRITTEN
            if found is None or found.strength is None:
                tally["missed"] += 1
                print(f"  missed {ev['id']:<8} {op:<8} ...{folder[-60:]}")
    n = tally["gold"] or 1
    print(f"labelled folders outside the cwd: {tally['gold']}")
    for k, name in (("command", "from the command alone"), ("session", "from the session, any evidence"),
                    ("session_written", "from the session, written or created")):
        print(f"  {name:<38} {tally[k]:>4}/{tally['gold']}  ({tally[k] / n:.0%})")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--calls", required=True)
    ap.add_argument("--sample", required=True)
    ap.add_argument("--labels", required=True)
    evaluate(ap.parse_args())
