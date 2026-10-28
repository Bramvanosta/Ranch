// A real BullMQ worker for Ranch's scheduler compatibility test.
//
//   node scheduler.mjs <major: 5|6> <queue>
//
// Driven over stdio by `src-tauri/tests/scheduler_compat_live.rs`. Ranch
// creates, edits, pauses, resumes and removes job schedulers; this process
// says what BullMQ makes of them and which jobs a real Worker receives:
//
//   `READY`                       → Ranch creates two schedulers → GO
//   `SCHEDULERS [...]`            BullMQ's own getJobSchedulers()
//   (worker runs 3.5 s) `RUNS_A`  → Ranch edits one, pauses the other → GO
//   (worker runs 4.5 s) `RUNS_B`  → Ranch resumes one, removes the other → GO
//   (worker runs 3.5 s) `RUNS_C`, `FINAL [...]`, `DONE`
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
}, 40_000)

const lines = createInterface({ input: process.stdin })
const go = () => new Promise((resolve) => lines.once("line", resolve))
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

const queue = new Queue(queueName, { connection, prefix })
await queue.waitUntilReady()
const describe = async () =>
  (await queue.getJobSchedulers()).map((s) => ({
    id: s.key ?? s.id,
    name: s.name,
    pattern: s.pattern ?? null,
    every: s.every ? Number(s.every) : null,
    tz: s.tz ?? null,
    next: s.next ?? null,
    data: s.template?.data ?? null,
  }))

out("READY")
await go()
out("SCHEDULERS", await describe())

let runs = []
const worker = new Worker(
  queueName,
  async (job) => {
    runs.push({
      name: job.name,
      id: job.id,
      data: job.data,
      scheduler: job.repeatJobKey ?? null,
      at: Date.now(),
    })
  },
  { connection, prefix, concurrency: 2 },
)
worker.on("error", (e) => out("WORKER_ERROR", { message: e.message }))
worker.on("failed", (job, e) => out("WORKER_ERROR", { job: job?.id, message: e.message }))

const stage = async (tag, ms) => {
  runs = []
  await sleep(ms)
  out(tag, runs)
}

await stage("RUNS_A", 3_500)
await go()
await stage("RUNS_B", 4_500)
await go()
await stage("RUNS_C", 3_500)
out("FINAL", await describe())

clearTimeout(timer)
await worker.close()
await queue.obliterate({ force: true })
await queue.close()
out("DONE")
process.exit(0)
