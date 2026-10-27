// A real BullMQ worker for Ranch's compatibility tests.
//
//   node worker.mjs <major: 5|6> <queue> <expected jobs> [timeout ms]
//
// Driven over stdio by `src-tauri/tests/add_job_compat_live.rs`:
//
//   1. creates the Queue (so its `meta` exists, as in a real app) → `READY`
//   2. waits for `GO` on stdin — Ranch adds its jobs meanwhile
//   3. prints `COUNTS {...}` and `PARSED [...]` — what BullMQ's own API
//      reads back from what Ranch wrote
//   4. starts a Worker (concurrency 1) and prints `JOB {...}` per job
//   5. after <expected> jobs: `EVENTS [...]` (what QueueEvents saw), `FINAL`
//      counts, obliterates the queue, then `DONE`
//
// Exit code 0 = all expected jobs processed; 2 = timeout; 1 = crash.
import { createInterface } from "node:readline"

const [major, queueName, expectedArg, timeoutArg] = process.argv.slice(2)
const expected = Number(expectedArg)
const timeoutMs = Number(timeoutArg ?? 20_000)
const { Queue, Worker, QueueEvents } = await import(`bullmq${major}`)

const connection = {
  host: process.env.RANCH_REDIS_HOST ?? "127.0.0.1",
  port: Number(process.env.RANCH_REDIS_PORT ?? 6390),
}
const prefix = process.env.RANCH_COMPAT_PREFIX ?? "ranchcompat"
const out = (tag, value) =>
  console.log(value === undefined ? tag : `${tag} ${JSON.stringify(value)}`)

const queue = new Queue(queueName, { connection, prefix })
const events = new QueueEvents(queueName, { connection, prefix })
await Promise.all([queue.waitUntilReady(), events.waitUntilReady()])
const seen = []
for (const name of ["added", "waiting", "delayed", "active", "completed"]) {
  events.on(name, (args) => seen.push({ event: name, jobId: args.jobId }))
}

const timer = setTimeout(async () => {
  out("TIMEOUT", { counts: await queue.getJobCounts() })
  process.exit(2)
}, timeoutMs)

out("READY", { bullmq: (await queue.getVersion?.()) ?? null })

const lines = createInterface({ input: process.stdin })
await new Promise((resolve) => lines.once("line", resolve))

out("COUNTS", await queue.getJobCounts())
const parsed = await queue.getJobs(["waiting", "paused", "prioritized", "delayed"])
out(
  "PARSED",
  parsed.map((j) => ({
    id: j.id,
    name: j.name,
    data: j.data,
    opts: j.opts,
    delay: j.delay,
    priority: j.priority,
  })),
)

let processed = 0
const worker = new Worker(
  queueName,
  async (job) => {
    out("JOB", {
      id: job.id,
      name: job.name,
      data: job.data,
      opts: job.opts,
      timestamp: job.timestamp,
      processedOn: job.processedOn,
      attemptsMade: job.attemptsMade,
    })
    return { ok: true }
  },
  { connection, prefix, concurrency: 1 },
)
worker.on("completed", async () => {
  processed += 1
  if (processed < expected) return
  clearTimeout(timer)
  // Let the events stream catch up with the last completion.
  await new Promise((r) => setTimeout(r, 200))
  out("EVENTS", seen)
  out("FINAL", await queue.getJobCounts())
  await worker.close()
  await events.close()
  await queue.obliterate({ force: true })
  await queue.close()
  out("DONE")
  process.exit(0)
})
worker.on("error", (e) => out("WORKER_ERROR", { message: e.message }))
