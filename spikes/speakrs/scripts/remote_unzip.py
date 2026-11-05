"""Extract named members from a remote zip via HTTP range requests (avoids the 2 GB VoxConverse download).

usage: python3 remote_unzip.py <zip-url> <out-dir> <member-suffix>...
"""

import sys
import urllib.request
import zipfile


class HttpFile:
    def __init__(self, url):
        self.url, self.pos, self.buf = url, 0, (0, b"")
        with urllib.request.urlopen(urllib.request.Request(url, method="HEAD")) as r:
            self.size = int(r.headers["Content-Length"])

    def seekable(self):
        return True

    def readable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, off, whence=0):
        self.pos = {0: off, 1: self.pos + off, 2: self.size + off}[whence]
        return self.pos

    def read(self, n=-1):
        end = self.size if n < 0 else min(self.size, self.pos + n)
        if end <= self.pos:
            return b""
        b0, buf = self.buf
        if not (b0 <= self.pos and end <= b0 + len(buf)):
            # ponytail: one 1 MB read-ahead buffer; enough for zipfile's small header reads
            stop = min(self.size, max(end, self.pos + (1 << 20)))
            req = urllib.request.Request(self.url, headers={"Range": f"bytes={self.pos}-{stop - 1}"})
            with urllib.request.urlopen(req) as r:
                self.buf = b0, buf = self.pos, r.read()
        data = buf[self.pos - b0 : end - b0]
        self.pos += len(data)
        return data


url, out, *wanted = sys.argv[1:]
zf = zipfile.ZipFile(HttpFile(url))
for info in zf.infolist():
    if any(info.filename.endswith(w) for w in wanted):
        info.filename = info.filename.rsplit("/", 1)[-1]
        zf.extract(info, out)
        print("extracted", info.filename, info.file_size)
