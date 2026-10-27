import { type Connection, commands, type FailureCause, type Job } from "@/lib/tauri/bindings"

/**
 * What Ask AI hands to an agent: the target, its last
 * error and stack, its last logs, the queue's state and a `ranch://` link,
 * then how to dig further with the Ranch MCP tools. Job data goes through
 * the `share_for_agents` policy first, as it would over MCP.
 */
export type AskTarget =
  | { kind: "job"; connection: Connection; job: Job }
  | { kind: "cause"; connection: Connection; cause: FailureCause; queueName?: string }
  | { kind: "connection"; connection: Connection }
  /** A prompt already written (a note's follow-up). */
  | { kind: "prompt"; connection: Connection | null; prompt: string }

const STACK_CAP = 4_000
const LOG_LINES = 20

const parse = (raw: string | null): unknown => {
  if (raw === null) return null
  try {
    return JSON.parse(raw)
  } catch {
    return raw
  }
}

const ranchLink = (connectionId: string, queue?: string, jobId?: string) =>
  `ranch://${encodeURIComponent(connectionId)}${queue ? `/queues/${encodeURIComponent(queue)}` : ""}${jobId ? `/jobs/${encodeURIComponent(jobId)}` : ""}`

const where = (c: Connection) =>
  `connection "${c.name}" (${c.envLabel}, ${c.host}:${c.port} db ${c.db})`

/** The job data as an agent may see it. */
async function shared(value: Record<string, unknown>): Promise<Record<string, unknown>> {
  const r = await commands.shareForAgents(JSON.stringify(value))
  if (r.status === "error") throw new Error(r.error.message)
  return JSON.parse(r.data) as Record<string, unknown>
}

async function queueState(connectionId: string, queueName: string): Promise<string> {
  const r = await commands.listQueues({ connectionId })
  if (r.status === "error") return "unknown (Redis did not answer)"
  const q = r.data.find((x) => x.queueRef.name === queueName)
  if (!q) return "unknown"
  const counts = Object.entries(q.counters)
    .filter(([, n]) => Number(n) > 0)
    .map(([state, n]) => `${state} ${n}`)
    .join(", ")
  return `${q.isPaused ? "paused" : "running"}; ${counts || "empty"}`
}

const FOOTER = [
  "Use the Ranch MCP tools to dig further (inspect_job, list_failed_jobs, list_failure_causes, get_job_logs, list_workers).",
  "Explain the cause and how to fix it. When you know, leave a note on the cause with annotate_cause so the next person sees it in Ranch.",
]

export async function buildPrompt(target: AskTarget): Promise<string> {
  switch (target.kind) {
    case "prompt":
      return target.prompt
    case "job": {
      const { connection, job } = target
      const queue = job.jobRef.queue.name
      const logs = await commands.getJobLogs({
        connectionId: connection.id,
        queueRef: job.jobRef.queue,
        jobId: job.jobRef.id,
        offset: 0,
        limit: 500,
      })
      const lines = logs.status === "ok" ? logs.data.lines.slice(-LOG_LINES) : []
      const data = await shared({
        data: parse(job.data),
        returnValue: parse(job.returnValue),
        progress: parse(job.progress),
        logs: lines,
      })
      const stack = job.stacktrace[0]?.slice(0, STACK_CAP)
      const sharedLogs = (data.logs as string[] | undefined) ?? []
      return [
        `Help me understand BullMQ job ${queue}#${job.jobRef.id} ("${job.name}") on ${where(connection)}.`,
        "",
        `State: ${job.state}, ${job.attemptsMade} attempt(s).`,
        job.failedReason ? `Last error: ${job.failedReason}` : "No error recorded.",
        stack ? `Stack (newest attempt):\n${stack}` : "",
        `Payload: ${JSON.stringify(data.data ?? null)}`,
        data.dataWithheld ? `(${String(data.dataWithheld)})` : "",
        sharedLogs.length > 0 ? `Last logs:\n${sharedLogs.join("\n")}` : "No logs.",
        `Queue ${queue}: ${await queueState(connection.id, queue)}.`,
        `Open in Ranch: ${ranchLink(connection.id, queue, job.jobRef.id)}`,
        "",
        ...FOOTER,
      ]
        .filter((line) => line !== "")
        .join("\n")
    }
    case "cause": {
      const { connection, cause } = target
      const queues = target.queueName ? [target.queueName] : cause.queueNames
      const states = await Promise.all(
        queues.slice(0, 5).map(async (q) => `- ${q}: ${await queueState(connection.id, q)}`),
      )
      const signature = cause.signature === "" ? "(no failure reason recorded)" : cause.signature
      return [
        `${cause.count} BullMQ job(s) fail with the same cause on ${where(connection)}:`,
        "",
        `Cause: ${signature}`,
        `Latest reason: ${cause.sample}`,
        `Job types: ${cause.jobNames.join(", ")}`,
        `Between ${new Date(cause.firstAt).toISOString()} and ${new Date(cause.lastAt).toISOString()}.`,
        `Some of its jobs: ${cause.jobs
          .slice(0, 10)
          .map((j) => `${j.queue.name}#${j.id}`)
          .join(", ")}`,
        "Queues:",
        ...states,
        `Open in Ranch: ${ranchLink(connection.id, queues[0])}`,
        "",
        ...FOOTER,
      ].join("\n")
    }
    case "connection": {
      const { connection } = target
      const r = await commands.listQueues({ connectionId: connection.id })
      const queues = r.status === "ok" ? r.data : []
      const causes = await commands.listFailureCauses({
        connectionId: connection.id,
        queue: null,
      })
      const top =
        causes.status === "ok"
          ? causes.data.causes
              .slice(0, 5)
              .map(
                (c) =>
                  `- ${c.count} × ${c.signature || "(no reason)"} (${c.queueNames.join(", ")})`,
              )
          : []
      return [
        `Help me check the health of the BullMQ queues on ${where(connection)}.`,
        "",
        "Queues:",
        ...queues.map(
          (q) =>
            `- ${q.queueRef.name}: ${q.isPaused ? "paused" : "running"}, waiting ${q.counters.waiting}, active ${q.counters.active}, failed ${q.counters.failed}`,
        ),
        top.length > 0 ? "Main failure causes:" : "No failures.",
        ...top,
        `Open in Ranch: ${ranchLink(connection.id)}`,
        "",
        ...FOOTER,
      ].join("\n")
    }
  }
}
