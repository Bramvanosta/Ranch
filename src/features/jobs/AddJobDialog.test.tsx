/**
 * Add job dialog: validates before sending, sends what was
 * typed, reports the result, and shows a refusal where the user is looking.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
vi.mock("@/lib/tauri/bindings", () => ({
  commands: { addJob: vi.fn() },
}))

import { toast } from "sonner"

import { commands } from "@/lib/tauri/bindings"

import { AddJobDialog } from "./AddJobDialog"

const mocked = commands as unknown as { addJob: ReturnType<typeof vi.fn> }

afterEach(() => {
  vi.resetAllMocks()
})

function renderDialog(props: Partial<React.ComponentProps<typeof AddJobDialog>> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const invalidate = vi.spyOn(client, "invalidateQueries")
  const onOpenChange = vi.fn()
  const onAdded = vi.fn()
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  render(
    <AddJobDialog
      open
      onOpenChange={onOpenChange}
      connectionId="c1"
      queueId={"bull:emails"}
      queueName="emails"
      queuePaused={false}
      onAdded={onAdded}
      {...props}
    />,
    { wrapper: Wrapper },
  )
  return { onOpenChange, onAdded, invalidate }
}

const type = (label: string, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } })

describe("AddJobDialog", () => {
  it("adds the job that was typed, refreshes the lists and hands the result over", async () => {
    mocked.addJob.mockResolvedValue({ status: "ok", data: { jobId: "42", state: "delayed" } })
    const { onOpenChange, onAdded, invalidate } = renderDialog()

    type("Job name", "send-email")
    type("Payload (JSON)", '{"to": "a@b.c"}')
    type("Delay (ms)", "5000")
    type("Attempts", "3")
    expect(screen.getByTestId("add-job-placement")).toHaveTextContent(/delayed/)
    fireEvent.click(screen.getByTestId("add-job-submit"))

    await waitFor(() => expect(mocked.addJob).toHaveBeenCalledTimes(1))
    expect(mocked.addJob).toHaveBeenCalledWith({
      connectionId: "c1",
      queueId: "bull:emails",
      job: {
        name: "send-email",
        data: '{"to": "a@b.c"}',
        delayMs: 5000,
        attempts: 3,
        priority: 0,
        lifo: false,
        backoff: null,
        jobId: null,
        removeOnComplete: false,
        removeOnFail: false,
      },
    })
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith({ jobId: "42", state: "delayed" }))
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(toast.success).toHaveBeenCalledWith(
      "Job #42 added to emails",
      expect.objectContaining({ description: "send-email · delayed" }),
    )
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["jobs", "c1"] })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["queues", "c1"] })
  })

  it("does not send an invalid form and says what is wrong, per field", async () => {
    renderDialog()
    type("Priority", "-4")
    fireEvent.click(screen.getByTestId("add-job-submit"))

    expect(await screen.findByTestId("add-job-name-error")).toHaveTextContent(/name/)
    expect(screen.getByTestId("add-job-priority-error")).toHaveTextContent(/1 = highest/)
    expect(mocked.addJob).not.toHaveBeenCalled()
  })

  it("invalid JSON: the button and ⌘↩ wait, and the error points at the line", () => {
    renderDialog()
    type("Job name", "x")
    type("Payload (JSON)", '{\n  "to": "a@b.c",\n}')
    expect(screen.getByTestId("add-job-submit")).toBeDisabled()
    expect(screen.getByTestId("add-job-data-error")).toHaveTextContent(
      "Line 3: No comma after the last property in JSON",
    )
    expect(screen.getByTestId("add-job-gutter").querySelector("[data-error]")).toHaveTextContent(
      "3",
    )
    fireEvent.keyDown(screen.getByLabelText("Payload (JSON)"), { key: "Enter", metaKey: true })
    expect(mocked.addJob).not.toHaveBeenCalled()
  })

  it("valid JSON says what it holds, and ⌘↩ adds the job", async () => {
    mocked.addJob.mockResolvedValue({ status: "ok", data: { jobId: "1", state: "waiting" } })
    renderDialog()
    type("Job name", "send-email")
    type("Payload (JSON)", '{"to": "a@b.c", "n": 1}')
    expect(screen.getByTestId("add-job-data-status")).toHaveTextContent(
      "Valid JSON · 2 keys · 20 bytes",
    )
    fireEvent.keyDown(screen.getByLabelText("Payload (JSON)"), { key: "Enter", metaKey: true })
    await waitFor(() => expect(mocked.addJob).toHaveBeenCalledTimes(1))
  })

  it("the options go out, and the same job shows as code", async () => {
    mocked.addJob.mockResolvedValue({ status: "ok", data: { jobId: "order-1", state: "waiting" } })
    renderDialog()
    type("Job name", "send-email")
    type("Payload (JSON)", "{}")
    type("Job ID", "order-1")
    fireEvent.click(screen.getByTestId("add-job-backoff-exponential"))
    type("Backoff delay (ms)", "1000")
    fireEvent.click(screen.getByLabelText("Keep when done"))
    expect(screen.getByTestId("add-job-snippet")).toHaveTextContent(
      'await queue.add("send-email", {}, { backoff: { type: "exponential", delay: 1000 }, jobId: "order-1", removeOnComplete: true, })',
    )
    fireEvent.click(screen.getByTestId("add-job-submit"))
    await waitFor(() =>
      expect(mocked.addJob).toHaveBeenCalledWith(
        expect.objectContaining({
          job: expect.objectContaining({
            backoff: { kind: "exponential", delayMs: 1000 },
            jobId: "order-1",
            removeOnComplete: true,
            removeOnFail: false,
          }),
        }),
      ),
    )
  })

  it("flags invalid JSON while typing and formats valid JSON on demand", () => {
    renderDialog()
    type("Payload (JSON)", "{oops")
    expect(screen.getByTestId("add-job-data-error")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Format" })).toBeDisabled()

    type("Payload (JSON)", '{"a":1}')
    expect(screen.queryByTestId("add-job-data-error")).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Format" }))
    expect((screen.getByLabelText("Payload (JSON)") as HTMLTextAreaElement).value).toBe(
      '{\n  "a": 1\n}',
    )
  })

  it("keeps the dialog open and shows the refusal (locked, queue gone…)", async () => {
    mocked.addJob.mockResolvedValue({
      status: "error",
      error: { kind: "connectionLocked", message: "Connection is locked", docUrl: null },
    })
    const { onOpenChange, onAdded } = renderDialog()
    type("Job name", "x")
    type("Payload (JSON)", "{}")
    fireEvent.click(screen.getByTestId("add-job-submit"))

    expect(await screen.findByRole("alert")).toHaveTextContent("Connection is locked")
    expect(onAdded).not.toHaveBeenCalled()
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
    // What was typed is still there.
    expect((screen.getByLabelText("Job name") as HTMLInputElement).value).toBe("x")
  })

  it("warns where the job will land on a paused queue", () => {
    renderDialog({ queuePaused: true })
    expect(screen.getByTestId("add-job-placement")).toHaveTextContent(/paused until it is resumed/)
  })
})
