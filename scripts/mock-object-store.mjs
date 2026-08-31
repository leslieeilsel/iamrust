import http from 'node:http';

const address = process.env.IAMRUST_MOCK_S3_ADDR ?? '127.0.0.1';
const port = Number(process.env.IAMRUST_MOCK_S3_PORT ?? '3900');
const objects = new Map();

const server = http.createServer((request, response) => {
  setCors(response);
  if (request.method === 'OPTIONS') {
    response.writeHead(204).end();
    return;
  }
  const url = new URL(request.url ?? '/', `http://${request.headers.host ?? 'localhost'}`);
  if (url.pathname === '/health') {
    response.writeHead(200, { 'content-type': 'application/json' }).end('{"status":"ok"}');
    return;
  }
  const key = decodeURIComponent(url.pathname);
  if (request.method === 'PUT') {
    const chunks = [];
    let size = 0;
    request.on('data', (chunk) => {
      size += chunk.length;
      if (size > 110 * 1024 * 1024) request.destroy();
      else chunks.push(chunk);
    });
    request.on('end', () => {
      objects.set(key, {
        body: Buffer.concat(chunks),
        contentType: request.headers['content-type'] ?? 'application/octet-stream',
        sha256: request.headers['x-amz-meta-sha256'],
      });
      response.writeHead(200, { etag: '"iamrust-e2e"' }).end();
    });
    return;
  }
  const object = objects.get(key);
  if (!object) {
    response.writeHead(404).end();
    return;
  }
  if (request.method === 'DELETE') {
    objects.delete(key);
    response.writeHead(204).end();
    return;
  }
  const headers = {
    'accept-ranges': 'bytes',
    'content-length': String(object.body.length),
    'content-type': object.contentType,
    ...(object.sha256 ? { 'x-amz-meta-sha256': object.sha256 } : {}),
  };
  if (request.method === 'HEAD') {
    response.writeHead(200, headers).end();
    return;
  }
  if (request.method === 'GET') {
    const range = /^bytes=(\d+)-(\d*)$/u.exec(request.headers.range ?? '');
    if (!range) {
      response.writeHead(200, headers).end(object.body);
      return;
    }
    const start = Number(range[1]);
    const requestedEnd = range[2] ? Number(range[2]) : object.body.length - 1;
    const end = Math.min(requestedEnd, object.body.length - 1);
    const body = object.body.subarray(start, end + 1);
    response
      .writeHead(206, {
        ...headers,
        'content-length': String(body.length),
        'content-range': `bytes ${start}-${end}/${object.body.length}`,
      })
      .end(body);
    return;
  }
  response.writeHead(405, { allow: 'PUT, HEAD, GET, DELETE, OPTIONS' }).end();
});

server.listen(port, address, () => {
  console.log(`I Am Rust mock object store listening on http://${address}:${port}`);
});

function setCors(response) {
  response.setHeader('access-control-allow-origin', '*');
  response.setHeader(
    'access-control-allow-headers',
    'content-type, range, x-amz-meta-sha256, x-requested-with',
  );
  response.setHeader('access-control-allow-methods', 'PUT, HEAD, GET, DELETE, OPTIONS');
  response.setHeader(
    'access-control-expose-headers',
    'content-length, content-range, content-type, x-amz-meta-sha256',
  );
}

function shutdown() {
  server.close(() => process.exit(0));
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
