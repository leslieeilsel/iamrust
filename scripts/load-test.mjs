const baseUrl = (
  process.env.IAMRUST_LOAD_TEST_URL ?? "http://127.0.0.1:3780"
).replace(/\/$/, "");
const concurrency = positiveInteger(
  process.env.IAMRUST_LOAD_TEST_CONCURRENCY,
  20,
);
const requestsPerWorker = positiveInteger(
  process.env.IAMRUST_LOAD_TEST_REQUESTS,
  100,
);
const mode = process.env.IAMRUST_LOAD_TEST_MODE ?? "messaging";

const result =
  mode === "health" ? await healthLoad() : await authenticatedMessagingLoad();
console.log(JSON.stringify(result, null, 2));
if (result.failures > 0 || result.p95_ms > 1_000) process.exitCode = 1;

async function healthLoad() {
  const latencies = [];
  let failures = 0;
  async function worker() {
    for (let index = 0; index < requestsPerWorker; index += 1) {
      const started = performance.now();
      try {
        await request("/health/live");
      } catch {
        failures += 1;
      }
      latencies.push(performance.now() - started);
    }
  }
  await Promise.all(Array.from({ length: concurrency }, () => worker()));
  return summarize("health", latencies, failures, {});
}

async function authenticatedMessagingLoad() {
  const suffix = crypto.randomUUID().replaceAll("-", "").slice(0, 12);
  const alice = await register(`load_alice_${suffix}`);
  const bob = await register(`load_bob_${suffix}`);
  const friendRequest = await request("/api/v1/friend-requests", {
    method: "POST",
    token: alice.access_token,
    body: { username: bob.profile.username, message: "load test" },
  });
  await request(`/api/v1/friend-requests/${friendRequest.id}`, {
    method: "PATCH",
    token: bob.access_token,
    body: { decision: "accept" },
  });
  const conversation = await request("/api/v1/conversations/direct", {
    method: "POST",
    token: alice.access_token,
    body: { peer_user_id: bob.profile.id },
  });

  const latencies = [];
  const expectedClientIds = new Set();
  let failures = 0;
  async function worker(workerIndex) {
    for (let index = 0; index < requestsPerWorker; index += 1) {
      const clientMessageId = crypto.randomUUID();
      expectedClientIds.add(clientMessageId);
      const started = performance.now();
      try {
        const ack = await request(
          `/api/v1/conversations/${conversation.id}/messages`,
          {
            method: "POST",
            token: alice.access_token,
            body: {
              client_message_id: clientMessageId,
              content: {
                type: "text",
                data: { text: `load ${workerIndex}:${index}` },
              },
              reply_to: null,
              mentions: [],
              mention_all: false,
              expires_in_seconds: null,
            },
          },
        );
        if (ack.client_message_id !== clientMessageId) failures += 1;
      } catch {
        failures += 1;
      }
      latencies.push(performance.now() - started);
    }
  }
  await Promise.all(
    Array.from({ length: concurrency }, (_, index) => worker(index)),
  );

  const syncedClientIds = new Set();
  let cursor = 0;
  let hasMore = true;
  while (hasMore) {
    const page = await request(`/api/v1/sync?after=${cursor}&limit=500`, {
      token: bob.access_token,
    });
    for (const event of page.events) {
      const clientId = event.payload?.message?.client_message_id;
      if (event.kind === "message_created" && typeof clientId === "string") {
        syncedClientIds.add(clientId);
      }
    }
    cursor = page.next_cursor;
    hasMore = page.has_more;
  }
  const missing = [...expectedClientIds].filter(
    (clientId) => !syncedClientIds.has(clientId),
  ).length;
  failures += missing;

  return summarize("messaging", latencies, failures, {
    concurrency,
    requests_per_worker: requestsPerWorker,
    synchronized_unique_messages: syncedClientIds.size,
    missing_messages: missing,
  });
}

async function register(username) {
  return request("/api/v1/auth/register", {
    method: "POST",
    body: {
      email: `${username}@example.test`,
      username,
      nickname: username,
      password: "LoadTestPass123",
      device_name: "load-test",
      platform: "node",
      app_version: "0.1.0",
    },
  });
}

async function request(path, options = {}) {
  const headers = { accept: "application/json" };
  if (options.token) headers.authorization = `Bearer ${options.token}`;
  if (options.body !== undefined) headers["content-type"] = "application/json";
  const response = await fetch(`${baseUrl}${path}`, {
    method: options.method ?? "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`request failed with ${response.status}: ${text.slice(0, 200)}`);
  }
  return text ? JSON.parse(text) : null;
}

function summarize(testMode, latencies, failures, extra) {
  latencies.sort((left, right) => left - right);
  const percentile = (value) =>
    latencies[
      Math.min(latencies.length - 1, Math.floor(latencies.length * value))
    ] ?? 0;
  return {
    mode: testMode,
    requests: latencies.length,
    failures,
    p50_ms: Math.round(percentile(0.5)),
    p95_ms: Math.round(percentile(0.95)),
    p99_ms: Math.round(percentile(0.99)),
    ...extra,
  };
}

function positiveInteger(value, fallback) {
  const parsed = Number(value ?? fallback);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error("load test parameters must be positive integers");
  }
  return parsed;
}
