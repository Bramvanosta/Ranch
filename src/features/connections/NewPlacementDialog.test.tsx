import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/tauri/bindings", () => ({
  commands: { createGroup: vi.fn(), createEnvironment: vi.fn() },
}))
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

import { commands } from "@/lib/tauri/bindings"

import { NewPlacementDialog } from "./NewPlacementDialog"

const mocked = commands as unknown as Record<string, ReturnType<typeof vi.fn>>

afterEach(() => vi.resetAllMocks())

function renderDialog(props: Partial<React.ComponentProps<typeof NewPlacementDialog>> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const onCreated = vi.fn()
  const onOpenChange = vi.fn()
  render(
    <QueryClientProvider client={client}>
      <NewPlacementDialog open onOpenChange={onOpenChange} onCreated={onCreated} {...props} />
    </QueryClientProvider>,
  )
  return { onCreated, onOpenChange }
}

describe("NewPlacementDialog", () => {
  it("creates an environment in an existing group; the name follows the color until typed", async () => {
    mocked.createEnvironment.mockResolvedValue({ status: "ok", data: "env-new" })
    const { onCreated, onOpenChange } = renderDialog({ group: { id: "g1", name: "Lumen" } })

    expect(screen.getByText("New environment in Lumen")).toBeInTheDocument()
    expect(screen.queryByLabelText("Group name")).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole("radio", { name: /Staging/i }))
    expect((screen.getByLabelText("Environment name") as HTMLInputElement).value).toBe("Staging")

    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    await waitFor(() =>
      expect(mocked.createEnvironment).toHaveBeenCalledWith({
        groupId: "g1",
        name: "Staging",
        colorToken: "env-staging",
      }),
    )
    expect(mocked.createGroup).not.toHaveBeenCalled()
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith("env-new"))
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it("without a group it creates the group first, then its first environment", async () => {
    mocked.createGroup.mockResolvedValue({ status: "ok", data: { id: "g-new" } })
    mocked.createEnvironment.mockResolvedValue({ status: "ok", data: "env-new" })
    const { onCreated } = renderDialog()

    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("Give the group a name")
    expect(mocked.createGroup).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText("Group name"), { target: { value: "Acme" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    await waitFor(() => expect(mocked.createGroup).toHaveBeenCalledWith({ name: "Acme" }))
    await waitFor(() =>
      expect(mocked.createEnvironment).toHaveBeenCalledWith({
        groupId: "g-new",
        name: "Local",
        colorToken: "env-local",
      }),
    )
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith("env-new"))
  })

  it("shows the backend's reason and stays open when creation fails", async () => {
    mocked.createEnvironment.mockResolvedValue({
      status: "error",
      error: {
        kind: "invalidInput",
        message: 'An environment named "Local" already exists in this group',
      },
    })
    const { onCreated, onOpenChange } = renderDialog({ group: { id: "g1", name: "Lumen" } })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("already exists")
    expect(onCreated).not.toHaveBeenCalled()
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
  })
})
