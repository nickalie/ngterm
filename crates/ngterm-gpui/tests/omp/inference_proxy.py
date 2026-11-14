import http.server
import itertools
import pathlib
import sys
import time
import urllib.error
import urllib.request

root = pathlib.Path(sys.argv[1])
upstream = sys.argv[2].rstrip('/')
sequence = itertools.count(1)


class Proxy(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        number = next(sequence)
        request = urllib.request.Request(
            upstream + self.path,
            data=self.rfile.read(int(self.headers['Content-Length'])),
            headers={'Content-Type': self.headers['Content-Type'], 'Authorization': self.headers.get('Authorization', '')},
            method='POST',
        )
        try:
            response = urllib.request.urlopen(request, timeout=180)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            body = response.read()
            status = response.status
            content_type = response.headers.get('Content-Type', 'application/octet-stream')
        (root / f'ready-{number}').touch()
        deadline = time.monotonic() + 240
        while not (root / f'release-{number}').exists():
            if time.monotonic() >= deadline:
                raise TimeoutError(f'inference gate {number} was not released')
            time.sleep(0.01)
        try:
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
pending = root / 'proxy-url.tmp'
pending.write_text(f'http://127.0.0.1:{server.server_port}')
pending.replace(root / 'proxy-url')
server.serve_forever()
