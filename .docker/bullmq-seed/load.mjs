// Continuous BullMQ activity on Ranch's Redis, for the slow-link bench
// (docs/slow-link.md): adds a job every 250 ms to one of six queues and processes
// them, with some failures. `node .docker/bullmq-seed/load.mjs` after the seed ran
// (it installs bullmq). REDIS_URL defaults to redis://127.0.0.1:6390.
import { Queue, Worker } from "bullmq"

const url = new URL(process.env.REDIS_URL || "redis://127.0.0.1:6390")
const connection = { host: url.hostname, port: Number(url.port || 6379) }
const names = ["emails", "images", "reports", "webhooks", "billing", "exports"]
const queues = names.map((n) => new Queue(n, { connection }))
for (const n of names) {
  new Worker(
    n,
    async () => {
      await new Promise((r) => setTimeout(r, 200 + Math.random() * 1500))
      if (Math.random() < 0.08) throw new Error(`upstream timeout after 30000ms (${n})`)
    },
    { connection, concurrency: 3 },
  )
}
setInterval(() => {
  const q = queues[Math.floor(Math.random() * queues.length)]
  q.add("task", { at: Date.now() }).catch(() => {})
}, 250)
console.log("load running")
