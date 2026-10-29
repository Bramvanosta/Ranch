/**
 * Scheduler sheet: next-runs preview from the backend parser,
 * inline refusal of an invalid pattern, create / edit / delete.
 */
import { QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { testQueryClient } from "@/test/renderWithProviders"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: {
    listCronJobs: vi.fn(),
    pauseScheduler: vi.fn(),
    resumeScheduler: vi.fn(),
    previewSchedule: vi.fn(),
    upsertScheduler: vi.fn(),
    removeScheduler: vi.fn(),
  },
}))

import { toast } from "sonner"

import { type CronJob, commands } from "@/lib/tauri/bindings"

import { CronJobsView } from "./CronJobsView"
import { SchedulerSheet } from "./SchedulerSheet"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>
const queueId = "bull:emails"
const RUNS = [1, 2, 3, 4, 5].map((d) => Date.UTC(2026, 8, 18 + d, 7, 0, 0))

afterEach(() => {
  vi.resetAllMocks()
})

function wrap(ui: ReactNode) {
  const client = testQueryClient()
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

const existing: CronJob = {
  key: "nightly",
  name: "build-report",
  pattern: "0 3 * * *",
  every: null,
  tz: "Europe/Paris",
  offset: 0,
  iterationCount: 12,
  nextRun: RUNS[0],
  endDate: null,
  template: '{"kind":"summary"}',
  paused: false,
}

const type = (label: string | RegExp, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } })

describe("SchedulerSheet: create", () => {
  it("previews the next 5 runs with the backend parser, then creates the scheduler", async () => {
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    mocked.upsertScheduler.mockResolvedValue({
      status: "ok",
      data: { schedulerId: "nightly", nextRunMs: RUNS[0], jobId: `repeat:nightly:${RUNS[0]}` },
    })
    const onOpenChange = vi.fn()
    wrap(
      <SchedulerSheet
        open
        onOpenChange={onOpenChange}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
      />,
    )

    type("Scheduler id", "nightly")
    type("Job name", "build-report")
    type("Cron pattern", "0 9 * * *")
    type("Timezone", "Europe/Paris")
    type("Job data template (JSON)", '{"kind":"summary"}')

    await waitFor(() =>
      expect(mocked.previewSchedule).toHaveBeenLastCalledWith({
        kind: "pattern",
        pattern: "0 9 * * *",
        tz: "Europe/Paris",
      }),
    )
    await waitFor(() =>
      expect(screen.getByTestId("scheduler-next-runs").querySelectorAll("li")).toHaveLength(5),
    )

    fireEvent.click(screen.getByTestId("scheduler-save"))
    await waitFor(() =>
      expect(mocked.upsertScheduler).toHaveBeenCalledWith({
        connectionId: "c1",
        queueId,
        scheduler: {
          id: "nightly",
          jobName: "build-report",
          schedule: { kind: "pattern", pattern: "0 9 * * *", tz: "Europe/Paris" },
          data: '{"kind":"summary"}',
        },
      }),
    )
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
    expect(toast.success).toHaveBeenCalledWith(
      "Scheduler nightly created",
      expect.objectContaining({ description: expect.stringContaining("0 9 * * * (Europe/Paris)") }),
    )
  })

  it("rejects an invalid pattern inline and blocks saving", async () => {
    mocked.previewSchedule.mockResolvedValue({
      status: "error",
      error: {
        kind: "invalidInput",
        message: "'L' is not supported in a cron pattern here",
        docUrl: null,
      },
    })
    wrap(
      <SchedulerSheet
        open
        onOpenChange={vi.fn()}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
      />,
    )
    type("Cron pattern", "0 9 L * *")
    expect(await screen.findByTestId("scheduler-pattern-error")).toHaveTextContent(
      "'L' is not supported",
    )
    expect(screen.getByTestId("scheduler-save")).toBeDisabled()
    expect(screen.getByTestId("scheduler-next-runs")).toHaveTextContent("Fix the schedule above")
  })

  it("switches to a fixed interval and sends it in milliseconds", async () => {
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    mocked.upsertScheduler.mockResolvedValue({
      status: "ok",
      data: { schedulerId: "tick", nextRunMs: RUNS[0], jobId: "repeat:tick:1" },
    })
    wrap(
      <SchedulerSheet
        open
        onOpenChange={vi.fn()}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
      />,
    )
    type("Scheduler id", "tick")
    type("Job name", "heartbeat")
    fireEvent.click(screen.getByLabelText("At a fixed interval"))
    type("Every", "15")
    fireEvent.change(screen.getByLabelText("Interval unit"), { target: { value: "minutes" } })
    await waitFor(() =>
      expect(mocked.previewSchedule).toHaveBeenLastCalledWith({ kind: "every", everyMs: 900_000 }),
    )
    fireEvent.click(screen.getByTestId("scheduler-save"))
    await waitFor(() =>
      expect(mocked.upsertScheduler).toHaveBeenCalledWith(
        expect.objectContaining({
          scheduler: expect.objectContaining({ schedule: { kind: "every", everyMs: 900_000 } }),
        }),
      ),
    )
  })

  it("does not send an incomplete form; shows a backend refusal in place", async () => {
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    const onOpenChange = vi.fn()
    wrap(
      <SchedulerSheet
        open
        onOpenChange={onOpenChange}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
      />,
    )
    fireEvent.click(screen.getByTestId("scheduler-save"))
    expect(await screen.findByTestId("scheduler-id-error")).toBeInTheDocument()
    expect(screen.getByTestId("scheduler-job-name-error")).toBeInTheDocument()
    expect(mocked.upsertScheduler).not.toHaveBeenCalled()

    mocked.upsertScheduler.mockResolvedValue({
      status: "error",
      error: { kind: "connectionLocked", message: "Connection is locked", docUrl: null },
    })
    type("Scheduler id", "nightly")
    type("Job name", "build-report")
    fireEvent.click(screen.getByTestId("scheduler-save"))
    expect(await screen.findByRole("alert")).toHaveTextContent("Connection is locked")
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
  })
})

describe("SchedulerSheet: edit & delete", () => {
  it("opens on the scheduler's settings, keeps its id, saves the change", async () => {
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    mocked.upsertScheduler.mockResolvedValue({
      status: "ok",
      data: { schedulerId: "nightly", nextRunMs: RUNS[0], jobId: "repeat:nightly:1" },
    })
    wrap(
      <SchedulerSheet
        open
        onOpenChange={vi.fn()}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
        scheduler={existing}
      />,
    )
    const id = screen.getByLabelText("Scheduler id") as HTMLInputElement
    expect(id.value).toBe("nightly")
    expect(id).toHaveAttribute("readonly")
    expect((screen.getByLabelText("Cron pattern") as HTMLInputElement).value).toBe("0 3 * * *")
    expect((screen.getByLabelText("Timezone") as HTMLInputElement).value).toBe("Europe/Paris")
    expect(
      (screen.getByLabelText("Job data template (JSON)") as HTMLTextAreaElement).value,
    ).toContain('"kind": "summary"')

    type("Cron pattern", "30 4 * * *")
    fireEvent.click(screen.getByTestId("scheduler-save"))
    await waitFor(() =>
      expect(mocked.upsertScheduler).toHaveBeenCalledWith(
        expect.objectContaining({
          scheduler: expect.objectContaining({
            id: "nightly",
            schedule: { kind: "pattern", pattern: "30 4 * * *", tz: "Europe/Paris" },
          }),
        }),
      ),
    )
    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith("Scheduler nightly updated", expect.anything()),
    )
  })

  it("deletes only after a second, explicit click", async () => {
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    mocked.removeScheduler.mockResolvedValue({
      status: "ok",
      data: { paused: false, alreadyInState: false },
    })
    const onOpenChange = vi.fn()
    wrap(
      <SchedulerSheet
        open
        onOpenChange={onOpenChange}
        connectionId="c1"
        queueId={queueId}
        queueName="emails"
        scheduler={existing}
      />,
    )
    const del = screen.getByTestId("scheduler-delete")
    fireEvent.click(del)
    expect(mocked.removeScheduler).not.toHaveBeenCalled()
    expect(del).toHaveTextContent(/Remove for good/)
    fireEvent.click(del)
    await waitFor(() =>
      expect(mocked.removeScheduler).toHaveBeenCalledWith({
        connectionId: "c1",
        queueId,
        schedulerKey: "nightly",
      }),
    )
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
  })
})

describe("CronJobsView", () => {
  it("offers New scheduler even when the queue has none", async () => {
    mocked.listCronJobs.mockResolvedValue({ status: "ok", data: [] })
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    wrap(<CronJobsView connectionId="c1" queueId={queueId} queueName="emails" />)
    fireEvent.click(await screen.findByTestId("scheduler-new"))
    expect(await screen.findByTestId("scheduler-sheet")).toHaveTextContent(
      "New scheduler in emails",
    )
  })

  it("opens the sheet on a row's Edit", async () => {
    mocked.listCronJobs.mockResolvedValue({ status: "ok", data: [existing] })
    mocked.previewSchedule.mockResolvedValue({ status: "ok", data: { nextRunsMs: RUNS } })
    wrap(<CronJobsView connectionId="c1" queueId={queueId} queueName="emails" />)
    fireEvent.click(await screen.findByTestId("cron-row-edit-nightly"))
    expect(await screen.findByTestId("scheduler-sheet")).toHaveTextContent("Edit scheduler nightly")
  })

  it("disables New and Edit on a locked connection", async () => {
    mocked.listCronJobs.mockResolvedValue({ status: "ok", data: [existing] })
    wrap(<CronJobsView connectionId="c1" queueId={queueId} queueName="emails" locked />)
    expect(await screen.findByTestId("scheduler-new")).toBeDisabled()
    expect(screen.getByTestId("cron-row-edit-nightly")).toBeDisabled()
  })
})
