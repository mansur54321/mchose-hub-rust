#!/usr/bin/env python3
"""Archive public M Hub modules with source URLs and hashes for protocol research."""
import concurrent.futures
import hashlib
import json
import pathlib
import re
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1] / "research/upstream"
BASE = "https://www.mchose.com.cn/"
ROOT.mkdir(parents=True, exist_ok=True)


def fetch(path):
    url = urllib.parse.urljoin(BASE, path)
    output = ROOT / (path.lstrip("/") or "index.html")
    if output.is_file():
        data = output.read_bytes()
    else:
        request = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(request, timeout=40) as response:
            data = response.read()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(data)
    text = data.decode("utf-8", errors="replace")
    refs = set()
    for match in re.findall(r'["\'`]([^"\'`\s]+\.(?:js|css))(?:[?][^"\'`\s]*)?["\'`]', text):
        if "${" in match:
            continue
        if match.startswith("assets/"):
            match = "/" + match
        resolved = urllib.parse.urlparse(urllib.parse.urljoin(url, match))
        if resolved.netloc == "www.mchose.com.cn" and re.fullmatch(r"/assets/[^/]+-[\w-]+\.(js|css)", resolved.path):
            refs.add(resolved.path)
    return {"url": url, "path": str(output.relative_to(ROOT)), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}, refs


def main():
    pending = {"/"}
    seen = set()
    manifest = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        while pending:
            batch = sorted(pending - seen)
            pending = set()
            if not batch:
                break
            seen.update(batch)
            for path, result in zip(batch, pool.map(fetch_safe, batch)):
                if isinstance(result, str):
                    manifest.append({"url": urllib.parse.urljoin(BASE, path), "error": result})
                else:
                    entry, refs = result
                    manifest.append(entry)
                    pending.update(refs - seen)
            print(f"Archived {len(manifest)} assets; next {len(pending)}", flush=True)
            (ROOT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2))


def fetch_safe(path):
    try:
        return fetch(path)
    except Exception as error:
        return str(error)


if __name__ == "__main__":
    main()
