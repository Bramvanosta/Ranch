import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import type { ComponentProps, ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

// Stub TanStack Router's Link so we don't need a router context.
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({
    to,
    params,
    children,
    ...rest
  }: {
    to: string
    params?: Record<string, string>
    children: ReactNode
  } & ComponentProps<"a">) => {
    const href = Object.entries(params ?? {}).reduce(
      (acc, [k, v]) => acc.replace(`$${k}`, encodeURIComponent(v)),
      to,
    )
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    )
  },
}))

vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    getJob: vi.fn(),
    getJobLogs: vi.fn(),
    // The drill-down asks for the job's flow to decide whether
    // to offer the Flow tab. Unset (undefined) here = "no flow".
    getFlow: vi.fn(),
    getQueueMetrics: vi.fn(),
  },
}))

import { commands, type Job } from "@/lib/tauri/bindings"

import { JobDrillDown } from "./JobDrillDown"

const mocked = commands as unknown as {
  getJob: ReturnType<typeof vi.fn>
  getJobLogs: ReturnType<typeof vi.fn>
  getFlow: ReturnType<typeof vi.fn>
}

afterEach(() => {
  vi.resetAllMocks()
})

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

const queueRef = { prefix: "bull", name: "emails" }

function makeJob(overrides: Partial<Job> = {}): Job {
  return {
    jobRef: { queue: queueRef, id: "42" },
    name: "send-email",
    state: "waiting",
    data: '{"to":"a@b.c"}',
    opts: '{"attempts":3}',
    attemptsMade: 0,
    timestamp: Date.now() - 60_000,
    processedOn: null,
    finishedOn: null,
    delay: null,
    failedReason: null,
    stacktrace: [],
    returnValue: null,
    parentKey: null,
    progress: null,
    ...overrides,
  }
}

describe("JobDrillDown", () => {
  it("on a failed job, the Trace tab opens first and the last error is in the header", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "failed",
        attemptsMade: 1,
        processedOn: Date.now() - 5_000,
        finishedOn: Date.now() - 4_000,
        failedReason: "Connection refused",
        stacktrace: ["Error: Connection refused", "    at fn"],
      }),
    })
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: [], total: 0, hasMore: false },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-trace")).toBeInTheDocument()
    expect(screen.getByRole("tab", { name: "Trace" })).toHaveAttribute("aria-selected", "true")
    expect(screen.getByTestId("job-last-error-reason")).toHaveTextContent("Connection refused")
    expect(screen.getByTestId("job-drill-down-copy-json")).toBeInTheDocument()
  })

  it("on a non-failed job, default tab is Payload and Copy Stack is hidden", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({ state: "waiting" }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-drill-down")).toBeInTheDocument()
    // Payload tab is active by default, with its own copy button (the header
    // no longer carries a row of "Copy …" buttons); nothing failed, so no
    // failure banner, no Stack trace tab, no Result tab.
    expect(screen.getByTestId("json-viewer-copy")).toBeInTheDocument()
    expect(screen.queryByTestId("job-failure-banner")).not.toBeInTheDocument()
    expect(screen.queryByTestId("job-drill-down-copy-stack")).not.toBeInTheDocument()
    expect(screen.queryByRole("tab", { name: "Stack trace" })).not.toBeInTheDocument()
    expect(screen.queryByRole("tab", { name: "Result" })).not.toBeInTheDocument()
  })

  it("jobNotFound error renders a 404-style empty state with back link", async () => {
    mocked.getJob.mockResolvedValue({
      status: "error",
      error: { kind: "jobNotFound", message: "Job not found: emails/999", docUrl: null },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="999"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-drill-down-not-found")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /Back to queue/ })).toHaveAttribute(
      "href",
      "/connections/c1/queues/emails",
    )
  })

  it("clicking a tab triggers onTabChange with the right value", async () => {
    const onTabChange = vi.fn()
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({ state: "waiting" }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"data"}
          onTabChange={onTabChange}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-drill-down")).toBeInTheDocument()
    fireEvent.click(screen.getByRole("tab", { name: "Raw" }))
    expect(onTabChange).toHaveBeenCalledWith("raw")
  })

  it("Result tab renders the parsed returnValue for a completed job", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "completed",
        returnValue: '{"ok":true,"messageId":"abc-123"}',
        processedOn: Date.now() - 5_000,
        finishedOn: Date.now() - 1_000,
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab="result"
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("result-tab")).toBeInTheDocument()
  })

  it("on a completed job with no return value, Result tab shows the empty state", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "completed",
        returnValue: null,
        processedOn: Date.now() - 5_000,
        finishedOn: Date.now() - 1_000,
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab="result"
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("result-tab-empty")).toHaveTextContent(
      /completed without emitting/i,
    )
  })

  it("a completed job defaults to the Result tab", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "completed",
        returnValue: '"hello"',
        processedOn: Date.now() - 5_000,
        finishedOn: Date.now() - 1_000,
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("result-tab")).toBeInTheDocument()
  })

  it("links in the old tab format open the tab that shows the same thing", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "completed",
        returnValue: '{"ok":true}',
        processedOn: Date.now() - 5_000,
        finishedOn: Date.now() - 1_000,
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"attempts"}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-trace")).toBeInTheDocument()
    expect(screen.getByTestId("job-details")).toBeInTheDocument()
  })

  it("the trace says how long the job waited and how long it ran", async () => {
    const t0 = Date.now() - 10_000
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "completed",
        attemptsMade: 1,
        timestamp: t0,
        processedOn: t0 + 2_000, // waited 2 s
        finishedOn: t0 + 6_000, // ran 4 s
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"trace"}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("trace-row-waiting")).toHaveTextContent("2.00 s")
    expect(screen.getByTestId("trace-row-attempt-1")).toHaveTextContent("completed · 4.00 s")
    expect(screen.getByTestId("job-trace-summary")).toHaveTextContent(
      "Added → completed in 6.00 s · 4.00 s of actual work",
    )
  })

  it("the details say where the job stands with its attempts, and leave out what does not apply", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "failed",
        attemptsMade: 3,
        opts: '{"attempts":3,"backoff":{"type":"exponential","delay":1000}}',
        failedReason: "boom",
        parentKey: "bull:orders:flow-1",
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    const details = await screen.findByTestId("job-details")
    expect(screen.getByTestId("job-details-attempts")).toHaveTextContent("3 of 3")
    expect(details).toHaveTextContent("exponential · 1 s")
    expect(screen.getByTestId("job-details-parent")).toHaveTextContent("orders #flow-1")
    expect(screen.getByTestId("job-details-parent")).toHaveAttribute(
      "href",
      "/connections/c1/queues/orders/flow-1",
    )
    // No priority, no delay, no progress on this job: no rows of dashes.
    expect(details).not.toHaveTextContent("Priority")
    expect(details).not.toHaveTextContent("Delay")
    expect(details).not.toHaveTextContent("Progress")
  })

  it("a failure is always on screen — with where the retries stand", async () => {
    const onTabChange = vi.fn()
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "failed",
        attemptsMade: 3,
        failedReason: "connect ETIMEDOUT",
        stacktrace: ["Error: connect ETIMEDOUT\n    at fn (/app/a.js:1:1)"],
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"data"}
          onTabChange={onTabChange}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-last-error-reason")).toHaveTextContent(
      "connect ETIMEDOUT",
    )
    expect(screen.getByTestId("job-last-error-status")).toHaveTextContent(
      "Failed after 3 of 3 attempts — no retry left.",
    )
    // The stack is one tab away.
    fireEvent.click(screen.getByRole("tab", { name: "Error" }))
    expect(onTabChange).toHaveBeenCalledWith("error")
  })

  it("the Error tab shows the whole error and its stack", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "failed",
        attemptsMade: 3,
        failedReason: "connect ETIMEDOUT",
        stacktrace: ["Error: connect ETIMEDOUT\n    at fn (/app/a.js:1:1)"],
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"stack"}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    const banner = await screen.findByTestId("job-failure-banner")
    expect(banner).toHaveAttribute("data-final", "true")
    expect(screen.getByTestId("stacktrace-viewer")).toBeInTheDocument()
    expect(screen.getByTestId("job-drill-down-copy-stack")).toBeInTheDocument()
  })

  it("a job between two attempts shows its last error as a warning, not as the end", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "delayed",
        attemptsMade: 1,
        processedOn: Date.now() - 30_000,
        delay: 2_000,
        failedReason: "429 Too Many Requests",
      }),
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"trace"}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-last-error-status")).toHaveTextContent(
      "Attempt 1 of 3 failed with this error — it will be retried.",
    )
    expect(screen.getByTestId("trace-row-pending")).toHaveTextContent("Retry scheduled")
  })

  it("Raw shows the whole job as JSON or YAML", async () => {
    mocked.getJob.mockResolvedValue({ status: "ok", data: makeJob() })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={"raw"}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    const text = await screen.findByTestId("job-raw-text")
    expect(text.textContent).toContain('"to": "a@b.c"')
    fireEvent.click(screen.getByTestId("job-raw-yaml"))
    expect(screen.getByTestId("job-raw-text").textContent).toContain("data:\n  to: a@b.c")
  })

  it("the Logs tab carries its line count", async () => {
    mocked.getJob.mockResolvedValue({ status: "ok", data: makeJob() })
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: ["a", "b", "c"], total: 3, hasMore: false },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByTestId("job-drill-down-log-count")).toHaveTextContent("3")
  })

  // ── Flow tab ─────────────────────────────────────────────────────────
  it("offers a Flow tab only for a job that is part of a flow", async () => {
    const flowNode = (id: string, parentKey: string | null) => ({
      key: `bull:emails:${id}`,
      jobRef: { queue: queueRef, id },
      parentKey,
      name: `job-${id}`,
      state: "completed",
      timestamp: 1,
      durationMs: null,
      failedReason: null,
      missing: false,
    })
    mocked.getJob.mockResolvedValue({ status: "ok", data: makeJob() })
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: [], total: 0, hasMore: false },
    })
    // A parent has no `parentKey`: only its flow tells.
    mocked.getFlow.mockResolvedValue({
      status: "ok",
      data: {
        nodes: [flowNode("42", null), flowNode("43", "bull:emails:42")],
        rootKey: "bull:emails:42",
        focusKey: "bull:emails:42",
        truncated: false,
      },
    })
    const Wrapper = wrap()
    const view = render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    expect(await screen.findByRole("tab", { name: "Flow" })).toBeInTheDocument()
    view.unmount()

    mocked.getFlow.mockResolvedValue({
      status: "ok",
      data: {
        nodes: [flowNode("42", null)],
        rootKey: "bull:emails:42",
        focusKey: "bull:emails:42",
        truncated: false,
      },
    })
    const Alone = wrap()
    render(
      <Alone>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Alone>,
    )
    expect(await screen.findByTestId("job-drill-down")).toBeInTheDocument()
    expect(screen.queryByRole("tab", { name: "Flow" })).not.toBeInTheDocument()
  })

  it("a running job opens on its progress, worker, lock and logs", async () => {
    mocked.getJob.mockResolvedValue({
      status: "ok",
      data: makeJob({
        state: "active",
        processedOn: Date.now() - 60_000,
        progress: '{"percentage":30,"message":"page 3 of 10"}',
        processedBy: "worker-emails-2",
        lockTtlMs: 25_000,
      }),
    })
    mocked.getJobLogs.mockResolvedValue({
      status: "ok",
      data: { lines: ["started"], total: 1, hasMore: false },
    })
    const Wrapper = wrap()
    render(
      <Wrapper>
        <JobDrillDown
          connectionId="c1"
          queueRef={queueRef}
          jobId="42"
          tab={undefined}
          onTabChange={vi.fn()}
        />
      </Wrapper>,
    )
    const panel = await screen.findByTestId("job-active")
    expect(screen.getByRole("tab", { name: "Progress" })).toHaveAttribute("aria-selected", "true")
    expect(panel).toHaveTextContent("page 3 of 10")
    expect(panel).toHaveTextContent("worker-emails-2")
    expect(screen.getByTestId("job-active-lock")).toHaveTextContent(
      "renewed ≈ 5.00 s ago · expires in 25.0 s",
    )
    expect(await screen.findByTestId("logs-tab")).toHaveTextContent("started")
  })
})
