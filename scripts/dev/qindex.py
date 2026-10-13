#!/usr/bin/env python3
"""Per-project semantic index over Qdrant (localhost:6333).

  qindex.py index [DIR]          create/refresh the collection for DIR's project (incremental)
  qindex.py search QUERY [-n N]  semantic search in the current project's collection
  qindex.py status [DIR]         show collection name and point count
  qindex.py refresh [DIR]        like index, but only if the project is indexed or a published index exists (silent otherwise)
  qindex.py export FILE [DIR]    write the project's collection to a portable file (gzip JSON lines)
  qindex.py import FILE [DIR]    load such a file into the project's collection, then run `index` to catch up
  qindex.py prune [--yes]        list (or with --yes delete) collections whose checkout no longer exists

A new checkout (a worktree) with no collection is seeded from a published index when the project's GitHub
repository has one: the newest unexpired Actions artifact named `qindex` from its default branch, fetched with
`gh`. Only files that differ from the published state are then embedded.
"""
import argparse
import base64
import fnmatch
import gzip
import hashlib
import json
import os
import struct
import re
import subprocess
import sys
import uuid
import zipfile
from pathlib import Path

from qdrant_client import QdrantClient, models

URL = "http://localhost:6333"
MODEL = "jinaai/jina-embeddings-v2-base-code"  # code-tuned, 30 languages, 8192-token context
DIM = 768
VEC = "fast-jina-embeddings-v2-base-code"
MODEL_TAG = "jc2"  # in the collection name: changing the model must change this (vectors are not compatible)
EXPORT_FORMAT = 1
ARTIFACT = "qindex"  # name of the published Actions artifact
CACHE = Path.home() / ".cache" / "qindex"
CHUNK_LINES, OVERLAP = 50, 10
MAX_BYTES = 500_000
SKIP_DIRS = {".git", "node_modules", ".venv", "venv", "__pycache__", "dist", "build", "target", ".next", "vendor", ".build", "Pods", "DerivedData", "Carthage", ".gradle", "cmake-build-debug"}
SKIP_NAMES = re.compile(r"(^\.env|\.pem$|\.key$|\.p12$|id_rsa|id_ed25519|credentials|\.lock$|-lock\.json$|\.min\.(js|css)$)", re.I)
TEXT_EXTS = {".md", ".txt", ".rst", ".py", ".js", ".jsx", ".ts", ".tsx", ".go", ".rs", ".java", ".kt", ".c", ".h",
             ".cpp", ".hpp", ".cs", ".rb", ".php", ".swift", ".sh", ".zsh", ".bash", ".sql", ".html", ".css", ".scss",
             ".json", ".yaml", ".yml", ".toml", ".ini", ".conf", ".cfg", ".xml", ".gradle", ".tf", ".proto", ".lua", ".cc", ".cxx", ".hh", ".hxx", ".inl", ".ipp", ".m", ".mm",
             ".kts", ".scala", ".groovy", ".cmake", ".mod", ".plist", ".entitlements", ".storyboard", ".pbxproj"}
TEXT_NAMES = {"Dockerfile", "Makefile", "compose.yaml", "docker-compose.yml", "CMakeLists.txt", "Package.swift", "Podfile", "go.mod"}
# Files that look like they hold a credential are not indexed. Quoted values must be opaque (no `$`, `\(`, `{`, `(`:
# those are shell variables or string interpolation, not secrets), and only a real `Authorization: Bearer|Basic`
# header counts (Swift's `authorization: SomeType` labels do not).
SECRET = re.compile(r"(sk-[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{30,}|AKIA[0-9A-Z]{16}|BEGIN [A-Z ]*PRIVATE KEY|"
                    r"(password|passwd|secret|api_?key|token)\s*[=:]\s*['\"][^'\"\s$\\{}()]{8,}['\"]|"
                    r"(?-i:Authorization):\s*(?:Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,})", re.I)
IGNORE_FILE = ".qindexignore"  # per-project patterns (fnmatch on the repo-relative path; "dir/" skips a folder)


def project_root(start):
    p = Path(start).expanduser().resolve()
    r = subprocess.run(["git", "-C", str(p), "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    return Path(r.stdout.strip()) if r.returncode == 0 else p


def collection_for(root):
    return f"proj-{re.sub(r'[^A-Za-z0-9_-]', '-', root.name)}-{hashlib.sha1(str(root).encode()).hexdigest()[:6]}-{MODEL_TAG}"


def ignore_patterns(root):
    try:
        lines = (root / IGNORE_FILE).read_text().splitlines()
    except OSError:
        return []
    return [l.strip() for l in lines if l.strip() and not l.lstrip().startswith("#")]


def ignored(rel, patterns):
    for pat in patterns:
        if pat.endswith("/"):
            if rel.startswith(pat) or fnmatch.fnmatch(rel, pat + "*"):
                return True
        elif fnmatch.fnmatch(rel, pat):
            return True
    return False


def list_files(root):
    r = subprocess.run(["git", "-C", str(root), "ls-files", "-co", "--exclude-standard"], capture_output=True, text=True)
    paths = [root / f for f in r.stdout.splitlines()] if r.returncode == 0 else [p for p in root.rglob("*") if p.is_file()]
    patterns = ignore_patterns(root)
    for p in paths:
        rel = p.relative_to(root)
        if SKIP_DIRS & set(rel.parts) or SKIP_NAMES.search(p.name) or not p.is_file() or ignored(rel.as_posix(), patterns):
            continue
        if p.suffix.lower() in TEXT_EXTS or p.name in TEXT_NAMES:
            yield p


MAX_CHARS = 2400      # target ceiling per chunk (~600 tokens)
CHUNKER_VER = b"ast-v2"  # mixed into file hashes so a chunking change re-embeds every file
LANGS = {".swift": "swift", ".go": "go", ".java": "java", ".c": "c", ".h": "c", ".cpp": "cpp", ".hpp": "cpp",
         ".cc": "cpp", ".cxx": "cpp", ".hh": "cpp", ".hxx": "cpp", ".inl": "cpp", ".ipp": "cpp", ".m": "objc",
         ".mm": "objc", ".cs": "csharp", ".py": "python", ".js": "javascript", ".jsx": "javascript",
         ".ts": "typescript", ".tsx": "tsx", ".rs": "rust", ".kt": "kotlin", ".kts": "kotlin", ".rb": "ruby",
         ".php": "php", ".scala": "scala", ".sh": "bash", ".bash": "bash", ".lua": "lua"}
_parsers = {}


def get_parser(lang):
    if lang not in _parsers:
        try:
            from tree_sitter_language_pack import get_parser as gp
            _parsers[lang] = gp(lang)
        except Exception:
            _parsers[lang] = None
    return _parsers[lang]


def line_spans(data, start, end):
    """Fallback: split data[start:end] into spans of at most CHUNK_LINES lines and MAX_CHARS bytes (byte offsets).
    Neighbouring spans overlap by up to OVERLAP lines; a single line longer than MAX_CHARS is cut into pieces.
    The size cap matters: embedding memory grows with the square of a chunk's length (a 5k-token chunk needs ~9 GB)."""
    offs = [start] + [i + 1 for i in range(start, end) if data[i] == 10 and i + 1 < end]
    n = len(offs)
    offs.append(end)
    out, i = [], 0
    while i < n:
        j = i + 1
        while j < n and j - i < CHUNK_LINES and offs[j + 1] - offs[i] <= MAX_CHARS:
            j += 1
        s, e = offs[i], offs[j]
        if e - s > MAX_CHARS:
            out.extend((k, min(k + MAX_CHARS, e), ()) for k in range(s, e, MAX_CHARS))
        else:
            out.append((s, e, ()))
        if j >= n:
            break
        i = max(j - min(OVERLAP, (j - i) // 5), i + 1)
    return out


def ast_spans(node, data, anc):
    """Recursively split a syntax node into (start, end, ancestor-headers) spans no bigger than MAX_CHARS."""
    if node.end_byte - node.start_byte <= MAX_CHARS:
        return [(node.start_byte, node.end_byte, anc)]
    kids = node.children
    if not kids:
        return line_spans(data, node.start_byte, node.end_byte)
    header = data[node.start_byte:node.end_byte].split(b"\n", 1)[0].decode("utf-8", "ignore").strip()[:100]
    useful = node.parent is not None and sum(ch.isalnum() for ch in header) >= 3  # skip root and bare "{" bodies
    inner = anc + ((node.start_byte, header),) if useful else anc
    spans, cursor = [], node.start_byte
    for k in kids:
        sub = ast_spans(k, data, inner)
        sub[0] = (cursor, sub[0][1], sub[0][2])  # gap text (comments, blank lines) rides with the next node
        spans.extend(sub)
        cursor = sub[-1][1]
    if spans and cursor < node.end_byte:
        spans[-1] = (spans[-1][0], node.end_byte, spans[-1][2])
    return spans


def merge_spans(spans):
    """Greedily join adjacent spans while the result stays under MAX_CHARS."""
    out = []
    for sp in spans:
        if out and sp[1] - out[-1][0] <= MAX_CHARS:
            out[-1] = (out[-1][0], sp[1], out[-1][2])
        else:
            out.append(sp)
    return out


def chunk_file(path, data):
    """Return [(start_line, end_line, text)] for a file. Code is split at syntax boundaries."""
    lang = LANGS.get(path.suffix.lower())
    parser = get_parser(lang) if lang else None
    spans = None
    if parser:
        try:
            tree = parser.parse(data)
            spans = merge_spans(ast_spans(tree.root_node, data, ()))
        except Exception:
            spans = None
    if spans is None:
        spans = line_spans(data, 0, len(data))
    out = []
    for st, en, anc in spans:
        body = data[st:en].decode("utf-8", "ignore").strip()
        if not body:
            continue
        ctx = [h for pos, h in anc if pos < st]
        if ctx:
            body = "// in: " + " > ".join(ctx) + "\n" + body
        out.append((data.count(b"\n", 0, st) + 1, data.count(b"\n", 0, en) + (0 if data[en - 1:en] == b"\n" else 1), body))
    return out


def connect():
    c = QdrantClient(url=URL, timeout=30)
    try:
        c.get_collections()
    except Exception:
        sys.exit("Qdrant not reachable at localhost:6333. Start it: docker start qdrant")
    return c


def create_collection(c, name):
    c.create_collection(name, vectors_config={VEC: models.VectorParams(size=DIM, distance=models.Distance.COSINE)})
    c.create_payload_index(name, "path", models.PayloadSchemaType.KEYWORD)


def export_header():
    return {"format": EXPORT_FORMAT, "model": MODEL, "model_tag": MODEL_TAG, "chunker": CHUNKER_VER.decode(), "dim": DIM}


def register(name, root):
    """Remember which checkout a collection belongs to, for `prune`."""
    f = CACHE / "collections.json"
    try:
        reg = json.loads(f.read_text())
    except (OSError, ValueError):
        reg = {}
    if reg.get(name) != str(root):
        reg[name] = str(root)
        CACHE.mkdir(parents=True, exist_ok=True)
        f.write_text(json.dumps(reg, indent=1, sort_keys=True))


def compatible_header(path):
    """The export's header if it was made with this model and chunker, else None (with the reason on stderr)."""
    try:
        with gzip.open(path, "rt", encoding="utf-8") as f:
            head = json.loads(f.readline())
    except (OSError, ValueError, EOFError) as e:
        print(f"Index file {path} is unreadable: {e}", file=sys.stderr)
        return None
    want = export_header()
    diff = [k for k in want if head.get(k) != want[k]]
    if diff:
        print(f"Index file not used: it differs in {', '.join(diff)} "
              f"({', '.join(f'{k}={head.get(k)!r}' for k in diff)}).", file=sys.stderr)
        return None
    return head


def import_file(c, name, path):
    """Load an exported index into a new collection `name`. Returns the header, or None if it is incompatible."""
    head = compatible_header(path)
    if head is None:
        return None
    with gzip.open(path, "rt", encoding="utf-8") as f:
        f.readline()
        create_collection(c, name)
        batch = []
        for line in f:
            r = json.loads(line)
            raw = base64.b64decode(r["v"])
            batch.append(models.PointStruct(id=r["id"], vector={VEC: list(struct.unpack(f"<{len(raw) // 4}f", raw))},
                                            payload=r["p"]))
            if len(batch) == 256:
                c.upsert(name, points=batch)
                batch = []
        if batch:
            c.upsert(name, points=batch)
    return head


def github_repo(root):
    r = subprocess.run(["git", "-C", str(root), "remote", "get-url", "origin"], capture_output=True, text=True)
    m = re.search(r"github\.com[:/]([^/]+)/([^/]+?)(?:\.git)?/?$", r.stdout.strip())
    return f"{m.group(1)}/{m.group(2)}" if m else None


def fetch_published(root):
    """Download the newest published index of root's GitHub repository. Returns a local file path or None."""
    repo = github_repo(root)
    if not repo:
        return None
    def gh(*args, **kw):
        return subprocess.run(["gh", "api", *args], capture_output=True, **kw)
    try:
        r = gh(f"repos/{repo}", "--jq", ".default_branch", text=True)
        if r.returncode:
            return None
        branch = r.stdout.strip()
        r = gh(f"repos/{repo}/actions/artifacts?name={ARTIFACT}&per_page=30", text=True)
        if r.returncode:
            return None
        arts = [x for x in json.loads(r.stdout).get("artifacts", [])
                if not x.get("expired") and (x.get("workflow_run") or {}).get("head_branch") == branch]
    except (OSError, ValueError):  # gh missing or bad JSON
        return None
    if not arts:
        return None
    art = max(arts, key=lambda x: x["created_at"])
    dest = CACHE / f"{repo.replace('/', '-')}-{art['id']}.jsonl.gz"
    if not dest.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        z = gh(f"repos/{repo}/actions/artifacts/{art['id']}/zip")
        if z.returncode:
            print(f"Could not download the published index: {z.stderr.decode(errors='ignore').strip()}", file=sys.stderr)
            return None
        tmp = dest.with_suffix(".zip")
        tmp.write_bytes(z.stdout)
        try:
            with zipfile.ZipFile(tmp) as zf:
                member = next(n for n in zf.namelist() if n.endswith(".jsonl.gz"))
                dest.write_bytes(zf.read(member))
        except (zipfile.BadZipFile, StopIteration):
            print("The published index artifact holds no .jsonl.gz file.", file=sys.stderr)
            return None
        finally:
            tmp.unlink(missing_ok=True)
    return dest


def seed(c, name, root):
    """Create collection `name` from the published index, if there is a compatible one. Returns True if seeded."""
    path = fetch_published(root)
    if not path:
        return False
    head = import_file(c, name, path)
    if head is None:
        return False
    print(f"Seeded from the published index (commit {str(head.get('commit', '?'))[:10]}, {c.count(name).count} points).")
    return True


def cmd_index(a):
    from fastembed import TextEmbedding
    root = project_root(a.dir)
    name = collection_for(root)
    c = connect()
    if not c.collection_exists(name) and not (not getattr(a, "no_seed", False) and seed(c, name, root)):
        create_collection(c, name)
    register(name, root)

    existing, offset = {}, None
    while True:
        pts, offset = c.scroll(name, limit=500, offset=offset, with_payload=["path", "file_hash"], with_vectors=False)
        for pt in pts:
            existing[pt.payload["path"]] = pt.payload.get("file_hash")
        if offset is None:
            break

    seen, todo, skipped_secret = set(), [], []
    for p in list_files(root):
        rel = str(p.relative_to(root))
        try:
            if p.stat().st_size > MAX_BYTES:
                continue
            data = p.read_bytes()
        except OSError:
            continue
        if b"\0" in data:
            continue
        text = data.decode("utf-8", errors="ignore")
        if SECRET.search(text):
            skipped_secret.append(rel)
            continue
        seen.add(rel)
        h = hashlib.sha1(data + CHUNKER_VER).hexdigest()
        if existing.get(rel) != h:
            todo.append((rel, h, data, p))

    stale = [r for r in existing if r not in seen]
    for rel in stale + [t[0] for t in todo]:
        if rel in existing:
            c.delete(name, points_selector=models.FilterSelector(
                filter=models.Filter(must=[models.FieldCondition(key="path", match=models.MatchValue(value=rel))])))

    emb = TextEmbedding(MODEL)
    n_chunks = 0
    if len(todo) > 50:
        print(f"Embedding {len(todo)} files...", flush=True)
    for k, (rel, h, data, p) in enumerate(todo, 1):
        if len(todo) > 50 and k % 50 == 0:
            print(f"  {k}/{len(todo)} files", flush=True)
        pieces = chunk_file(p, data)
        docs = [f"{rel}\n{t}" for _, _, t in pieces]
        metas = [(a, b) for a, b, _ in pieces]
        if not docs:
            continue
        vecs = list(emb.embed(docs, batch_size=16))  # bounded batches keep peak memory predictable
        c.upsert(name, points=[
            models.PointStruct(
                id=str(uuid.UUID(hashlib.sha1(f"{rel}:{i}".encode()).hexdigest()[:32])),
                vector={VEC: v.tolist()},
                payload={"document": d, "path": rel, "file_hash": h, "start_line": s, "end_line": e,
                         "metadata": {"path": rel, "lines": f"{s}-{e}"}})
            for i, (v, d, (s, e)) in enumerate(zip(vecs, docs, metas))])
        n_chunks += len(docs)

    total = c.count(name).count
    print(f"Project: {root}\nCollection: {name}\nUpdated {len(todo)} files ({n_chunks} chunks), removed {len(stale)} stale, "
          f"{len(existing) - len(stale) - sum(1 for t in todo if t[0] in existing)} unchanged. Total points: {total}")
    if skipped_secret:
        print(f"Skipped {len(skipped_secret)} file(s) that look like they contain secrets:")
        for s in skipped_secret[:20]:
            print(f"  {s}")


def cmd_search(a):
    from fastembed import TextEmbedding
    root = project_root(a.dir)
    name = collection_for(root)
    c = connect()
    if not c.collection_exists(name):
        sys.exit(f"No index for {root}. Run: qindex.py index")
    q = list(TextEmbedding(MODEL).embed([a.query]))[0].tolist()
    for h in c.query_points(name, query=q, using=VEC, limit=a.n).points:
        p = h.payload
        body = p["document"].split("\n", 1)[1] if "\n" in p["document"] else p["document"]
        print(f"--- {p['path']}:{p['start_line']}-{p['end_line']}  (score {h.score:.2f})")
        print("\n".join(body.splitlines()[:a.lines]))


def cmd_refresh(a):
    root = project_root(a.dir)
    name = collection_for(root)
    try:
        c = QdrantClient(url=URL, timeout=5)
        if not c.collection_exists(name) and not seed(c, name, root):
            return  # never build a whole index unasked; seeding from a published one is cheap
    except Exception:
        return
    cmd_index(a)


def cmd_export(a):
    root = project_root(a.dir)
    name = collection_for(root)
    c = connect()
    if not c.collection_exists(name):
        sys.exit(f"No index for {root}. Run: qindex.py index")
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    n, offset = 0, None
    with gzip.open(a.file, "wt", encoding="utf-8") as f:
        f.write(json.dumps({**export_header(), "commit": commit}) + "\n")
        while True:
            pts, offset = c.scroll(name, limit=256, offset=offset, with_payload=True, with_vectors=True)
            for pt in pts:
                v = pt.vector[VEC] if isinstance(pt.vector, dict) else pt.vector
                f.write(json.dumps({"id": str(pt.id), "v": base64.b64encode(struct.pack(f"<{len(v)}f", *v)).decode(),
                                    "p": pt.payload}) + "\n")
                n += 1
            if offset is None:
                break
    print(f"Exported {n} points from {name} (commit {commit[:10]}) to {a.file}")


def cmd_import(a):
    root = project_root(a.dir)
    name = collection_for(root)
    c = connect()
    if compatible_header(a.file) is None:
        sys.exit(1)
    if c.collection_exists(name):
        if not a.replace:
            sys.exit(f"{name} already exists; pass --replace to overwrite it")
        c.delete_collection(name)
    import_file(c, name, a.file)
    register(name, root)
    print(f"Imported {c.count(name).count} points into {name}. Run `qindex.py index` to catch up with this checkout.")


def cmd_prune(a):
    c = connect()
    try:
        reg = json.loads((CACHE / "collections.json").read_text())
    except (OSError, ValueError):
        reg = {}
    gone, unknown = [], []
    for col in sorted(x.name for x in c.get_collections().collections if x.name.startswith("proj-")):
        root = reg.get(col)
        if root is None:
            unknown.append(col)
        elif not Path(root).is_dir():
            gone.append((col, root))
    for col, root in gone:
        if a.yes:
            c.delete_collection(col)
            reg.pop(col, None)
        print(f"{'deleted' if a.yes else 'stale'}: {col} ({root} no longer exists)")
    if a.yes:  # also forget checkouts whose collection was deleted some other way
        existing = {x.name for x in c.get_collections().collections}
        reg = {k: v for k, v in reg.items() if k in existing}
        CACHE.mkdir(parents=True, exist_ok=True)
        (CACHE / "collections.json").write_text(json.dumps(reg, indent=1, sort_keys=True))
    for col in unknown:
        print(f"unknown: {col} (indexed before prune tracked checkouts; re-index it or delete it by hand)")
    if gone and not a.yes:
        print("Run with --yes to delete the stale collections.")
    if not gone and not unknown:
        print("Nothing to prune.")


def cmd_status(a):
    root = project_root(a.dir)
    name = collection_for(root)
    c = connect()
    print(f"Project: {root}\nCollection: {name}")
    print(f"Points: {c.count(name).count}" if c.collection_exists(name) else "Not indexed")


ap = argparse.ArgumentParser()
sub = ap.add_subparsers(dest="cmd", required=True)
for nm, fn in (("index", cmd_index), ("status", cmd_status), ("refresh", cmd_refresh)):
    sp = sub.add_parser(nm); sp.add_argument("dir", nargs="?", default="."); sp.set_defaults(fn=fn)
    if nm == "index":
        sp.add_argument("--no-seed", action="store_true", help="do not seed a new collection from a published index")
for nm, fn in (("export", cmd_export), ("import", cmd_import)):
    sp = sub.add_parser(nm); sp.add_argument("file"); sp.add_argument("dir", nargs="?", default="."); sp.set_defaults(fn=fn)
    if nm == "import":
        sp.add_argument("--replace", action="store_true", help="overwrite an existing collection")
sp = sub.add_parser("prune"); sp.add_argument("--yes", action="store_true"); sp.set_defaults(fn=cmd_prune)
sp = sub.add_parser("search"); sp.add_argument("query"); sp.add_argument("-n", type=int, default=8)
sp.add_argument("--lines", type=int, default=12, help="snippet lines per hit"); sp.add_argument("--dir", default=".")
sp.set_defaults(fn=cmd_search)
a = ap.parse_args()
a.fn(a)
