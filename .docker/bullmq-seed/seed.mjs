// Seed local Redis with BullMQ queues + jobs so Ranch's queue/job views
// have realistic data to render. Idempotent in spirit:
// rerun adds more jobs but doesn't break anything.
//
// Run via `docker compose --profile bullmq up bullmq-seed`. The container
// runs `npm install && node seed.mjs` and exits.

import { Queue, Worker } from "bullmq"

const url = process.env.REDIS_URL || "redis://127.0.0.1:6379"
console.log(`Seeding BullMQ data into ${url}...`)

// Parse REDIS_URL into the connection shape BullMQ wants. BullMQ accepts
// either { host, port } or a IORedis-compatible URL via `connection`.
const u = new URL(url)
const connection = {
  host: u.hostname,
  port: Number(u.port || 6379),
}

// ── 3 queues, varied workloads ─────────────────────────────────────────

const emails = new Queue("emails", { connection })
const images = new Queue("images", { connection })
const reports = new Queue("reports", { connection })

// 50 waiting email jobs.
for (let i = 0; i < 50; i++) {
  await emails.add(
    "send-email",
    { to: `user${i}@example.com`, subject: `Welcome ${i}`, body: "Hi!" },
    { attempts: 3, backoff: { type: "exponential", delay: 1000 } },
  )
}

// 10 delayed reminders (visible in the `delayed` state).
for (let i = 0; i < 10; i++) {
  await emails.add(
    "send-reminder",
    { userId: `u-${i}`, type: "trial-ending" },
    { delay: 60_000 + i * 10_000, attempts: 2 },
  )
}

// 5 jobs we want to FAIL: we ack them via a synthetic worker that throws.
for (let i = 0; i < 5; i++) {
  await emails.add("send-broken", { to: `broken-${i}@example.com` }, { attempts: 1 })
}

// One distinct failed job that also writes BullMQ logs via
// `Job.log()`. The `get_job_logs` live test queries this specific job
// (matched by name `send-broken-with-logs`) and asserts the 3 known
// lines.
await emails.add("send-broken-with-logs", { to: "logs-target@example.com" }, { attempts: 1 })

// CRON / repeatable schedulers for the list_cron_jobs view.
// One cron-pattern scheduler + one fixed-interval scheduler so the live
// tests can assert both code paths.
await emails.upsertJobScheduler(
  "daily-report",
  { pattern: "0 9 * * *", tz: "Europe/Paris" },
  { name: "send-daily-report", data: { reportType: "summary" } },
)
await emails.upsertJobScheduler(
  "every-15m",
  { every: 15 * 60 * 1000 },
  { name: "ping-health", data: {} },
)

// 30 image-resize jobs, mostly waiting; some completed via a worker pass.
for (let i = 0; i < 30; i++) {
  await images.add("resize", {
    url: `s3://bucket/photo-${i}.jpg`,
    width: 800,
    height: 600,
  })
}

// 5 daily reports (waiting).
for (let i = 0; i < 5; i++) {
  await reports.add("daily-report", { reportId: `r-${i}`, date: "2026-05-09" })
}

// Process a handful synthetically so the queues have completed/failed states.
//
//   - `send-broken` jobs throw → end up in `failed`.
//   - The first 8 `resize` jobs succeed → end up in `completed`.
//
// We close each worker after it processes its quota so this seed exits.
console.log("Processing 5 send-broken (will fail) + 8 resize (will complete)...")

let resizeProcessed = 0
const imageWorker = new Worker(
  "images",
  async (job) => {
    if (job.name !== "resize" || resizeProcessed >= 8) {
      // Tell BullMQ we don't want this job; it stays waiting.
      throw new Error("seeder-skip")
    }
    resizeProcessed++
    return { resizedUrl: `${job.data.url.replace(".jpg", "")}-800.jpg`, sizeKb: 42 }
  },
  // This worker records BullMQ's native metrics (per-minute
  // counters in `bull:images:metrics:*`), so the `images` queue exercises the
  // exact-count path. The `emails` worker below deliberately does NOT, which
  // keeps the estimated (ZCOUNT) path covered too.
  { connection, concurrency: 1, metrics: { maxDataPoints: 60 * 24 } },
)

let brokenProcessed = 0
let loggedJobProcessed = false
const emailWorker = new Worker(
  "emails",
  async (job) => {
    // The one job that exercises BullMQ's per-job log list.
    if (job.name === "send-broken-with-logs") {
      await job.log("Attempting send...")
      await job.log("Network timeout")
      await job.log("Will retry")
      loggedJobProcessed = true
      throw new Error("Network timeout after 3 attempts (synthetic)")
    }
    if (job.name !== "send-broken") {
      throw new Error("seeder-skip")
    }
    brokenProcessed++
    throw new Error("Connection refused (synthetic)")
  },
  { connection, concurrency: 1 },
)

// Wait until all quotas are met (or 8 seconds, whichever comes first).
const start = Date.now()
while (resizeProcessed < 8 || brokenProcessed < 5 || !loggedJobProcessed) {
  if (Date.now() - start > 8_000) break
  await new Promise((r) => setTimeout(r, 200))
}

await imageWorker.close()
await emailWorker.close()
await emails.close()
await images.close()
await reports.close()

console.log(
  `✓ Seeded: emails (50 wait + 10 delay + 5 fail + 1 fail-with-logs + 2 schedulers), images (8 done + 22 wait), reports (5 wait)`,
)
process.exit(0)
