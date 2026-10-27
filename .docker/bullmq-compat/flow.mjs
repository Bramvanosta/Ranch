// A real BullMQ flow for Ranch's flow-reading test.
//
//   node flow.mjs <major: 5|6> <queue base name>
//
// Builds, with BullMQ's own FlowProducer, across two queues (`<base>-a`,
// `<base>-b`):
//
//   assemble (a)                      ← root, stays in waiting-children
//   ├── fetch-ok   (b)  → completed
//   ├── fetch-bad  (b)  → failed (the parent keeps waiting for it)
//   └── prepare    (b)                ← itself a parent, waiting-children
//       └── download (a) → completed… then `prepare` runs → completed
//
// then prints `READY {ids}`, waits for `GO`, obliterates both queues, `DONE`.
import { createInterface } from "node:readline"

const [major, base] = process.argv.slice(2)
const { FlowProducer, Queue, Worker } = await import(`bullmq${major}`)

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

const queueA = new Queue(`${base}-a`, { connection, prefix })
const queueB = new Queue(`${base}-b`, { connection, prefix })
await Promise.all([queueA.waitUntilReady(), queueB.waitUntilReady()])

const flows = new FlowProducer({ connection, prefix })
const tree = await flows.add({
  name: "assemble",
  queueName: `${base}-a`,
  data: { order: 4711 },
  children: [
    { name: "fetch-ok", queueName: `${base}-b`, data: { part: 1 } },
    { name: "fetch-bad", queueName: `${base}-b`, data: { part: 2 } },
    {
      name: "prepare",
      queueName: `${base}-b`,
      data: { part: 3 },
      children: [{ name: "download", queueName: `${base}-a`, data: { file: "x" } }],
    },
  ],
})

const process_ = async (job) => {
  if (job.name === "fetch-bad") throw new Error("upstream 503")
  return { done: job.name }
}
const workers = [
  new Worker(`${base}-a`, process_, { connection, prefix }),
  new Worker(`${base}-b`, process_, { connection, prefix }),
]
// download → prepare → (fetch-ok) completed; fetch-bad failed; assemble waits.
for (;;) {
  await new Promise((r) => setTimeout(r, 100))
  const [a, b] = await Promise.all([queueA.getJobCounts(), queueB.getJobCounts()])
  if (a.completed === 1 && b.completed === 2 && b.failed === 1) break
}
await Promise.all(workers.map((w) => w.close()))

const ids = { assemble: tree.job.id }
for (const child of tree.children) {
  ids[child.job.name] = child.job.id
  for (const grandchild of child.children ?? []) ids[grandchild.job.name] = grandchild.job.id
}
out("READY", ids)

const lines = createInterface({ input: process.stdin })
await new Promise((resolve) => lines.once("line", resolve))

clearTimeout(timer)
await flows.close()
await queueA.obliterate({ force: true })
await queueB.obliterate({ force: true })
await Promise.all([queueA.close(), queueB.close()])
out("DONE")
process.exit(0)
