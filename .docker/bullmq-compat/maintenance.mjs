// A real BullMQ producer + worker for Ranch's maintenance
// compatibility test.
//
//   node maintenance.mjs <major: 5|6> <queue>
//
// Driven over stdio by `src-tauri/tests/maintenance_compat_live.rs`:
//
//   1. adds 6 jobs that fail on their first run and 3 that succeed, works
//      them (6 failed, 3 completed), stops the worker      → `STAGE_A {counts}`
//   2. waits for `GO` — Ranch retries all failed meanwhile
//   3. prints `AFTER_RETRY {counts}`, works the queue again → `REPROCESSED {…}`
//   4. adds 4 waiting + 2 delayed + 1 prioritized jobs and a job scheduler
//                                                          → `STAGE_B {counts}`
//   5. waits for `GO` — Ranch drains, then cleans completed
//   6. prints `AFTER_CLEANUP {counts, schedulers}`, waits for a last `GO`
//      (Ranch inspects the raw keys), obliterates the queue → `DONE`
import { createInterface } from "node:readline"

const [major, queueName] = process.argv.slice(2)
const { Queue, Worker } = await import(`bullmq${major}`)

const connection = {
  host: process.env.RANCH_REDIS_HOST ?? "127.0.0.1",
  port: Number(process.env.RANCH_REDIS_PORT ?? 6390),
}
const prefix = process.env.RANCH_COMPAT_PREFIX ?? "ranchcompat"
const out = (tag, value) =>
  console.log(value === undefined ? tag : `${tag} ${JSON.stringify(value)}`)

const timer = setTimeout(() => {
  out("TIMEOUT")
  process.exit(2)
}, 30_000)

const lines = createInterface({ input: process.stdin })
const go = () => new Promise((resolve) => lines.once("line", resolve))

const queue = new Queue(queueName, { connection, prefix })
await queue.waitUntilReady()

/** Work the queue until `done(counts)` holds, then stop the worker. */
async function workUntil(done) {
  const runs = []
  const worker = new Worker(
    queueName,
    async (job) => {
      runs.push({ name: job.name, id: job.id, attemptsMade: job.attemptsMade })
      // Fails the first time it is ever run, succeeds once retried.
      if (job.name === "flaky" && job.attemptsMade === 0) throw new Error("first run fails")
      return { ok: true }
    },
    { connection, prefix, concurrency: 1 },
  )
  worker.on("error", (e) => out("WORKER_ERROR", { message: e.message }))
  for (;;) {
    await new Promise((r) => setTimeout(r, 100))
    if (done(await queue.getJobCounts())) break
  }
  await worker.close()
  return runs
}

// 1 ─────────────────────────────────────────────────────────────────────
for (let i = 0; i < 6; i++) await queue.add("flaky", { i })
for (let i = 0; i < 3; i++) await queue.add("steady", { i })
await workUntil((c) => c.failed === 6 && c.completed === 3)
out("STAGE_A", await queue.getJobCounts())

// 2–3 ───────────────────────────────────────────────────────────────────
await go()
out("AFTER_RETRY", await queue.getJobCounts())
const reruns = await workUntil((c) => c.completed === 9 || c.failed > 0)
out("REPROCESSED", { counts: await queue.getJobCounts(), reruns })

// 4 ─────────────────────────────────────────────────────────────────────
for (let i = 0; i < 4; i++) await queue.add("later-never", { i })
for (let i = 0; i < 2; i++) await queue.add("delayed-never", { i }, { delay: 3_600_000 })
await queue.add("vip-never", {}, { priority: 2 })
await queue.upsertJobScheduler("hourly", { every: 3_600_000 }, { name: "tick", data: {} })
out("STAGE_B", await queue.getJobCounts())

// 5–6 ───────────────────────────────────────────────────────────────────
await go()
const schedulers = await queue.getJobSchedulers()
out("AFTER_CLEANUP", {
  counts: await queue.getJobCounts(),
  schedulers: schedulers.map((s) => s.key ?? s.id ?? s.name),
})
// Ranch looks at the raw keys (orphans?) before the queue is obliterated.
await go()

clearTimeout(timer)
await queue.obliterate({ force: true })
await queue.close()
out("DONE")
process.exit(0)
