import { afterEach, describe, expect, it, vi } from "vitest"

import { CSRF_HEADER, httpInvoke, installHttpBackend } from "./httpBackend"
import { getRuntime, hasAgents, hasUpdater, setRuntime } from "./runtime"

const fetchMock = vi.fn()
vi.stubGlobal("fetch", fetchMock)

const respond = (status: number, body: unknown, statusText = "") =>
  new Response(body === undefined ? "" : JSON.stringify(body), { status, statusText })

afterEach(() => {
  fetchMock.mockReset()
  setRuntime("desktop")
})

describe("httpInvoke", () => {
  it("posts the command's arguments as Tauri would receive them, with the CSRF header", async () => {
    fetchMock.mockResolvedValue(respond(200, [{ id: "g1" }]))
    const result = await httpInvoke("list_queues", { args: { connectionId: "c1" } })
    expect(result).toEqual([{ id: "g1" }])
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe("/api/invoke/list_queues")
    expect(init.method).toBe("POST")
    expect(init.credentials).toBe("same-origin")
    expect((init.headers as Record<string, string>)[CSRF_HEADER]).toBe("1")
    expect(JSON.parse(init.body as string)).toEqual({ args: { connectionId: "c1" } })
  })

  it("a command that returns nothing resolves to null", async () => {
    fetchMock.mockResolvedValue(respond(200, null))
    await expect(httpInvoke("set_kv_setting", { key: "a", value: "b" })).resolves.toBeNull()
  })

  it("rejects with the server's typed error, as Tauri does", async () => {
    const dto = { kind: "connectionLocked", message: "Connection prod is locked", docUrl: null }
    fetchMock.mockResolvedValue(respond(423, dto))
    await expect(httpInvoke("remove_job", {})).rejects.toEqual(dto)
  })

  it("says so when something else answered in the server's place", async () => {
    fetchMock.mockResolvedValue(
      new Response("<html>Bad gateway</html>", { status: 502, statusText: "Bad Gateway" }),
    )
    await expect(httpInvoke("list_groups", {})).rejects.toMatchObject({
      kind: "http",
      message: "The Ranch server answered 502 Bad Gateway",
    })
    fetchMock.mockResolvedValue(new Response("", { status: 401 }))
    await expect(httpInvoke("list_groups", {})).rejects.toMatchObject({ kind: "unauthorized" })
  })

  it("signed out of an OIDC server: goes to sign in once, and comes back here", async () => {
    const assign = vi.fn()
    vi.stubGlobal("location", { pathname: "/connections/c1", search: "?view=queues", assign })
    const signedOut = () =>
      respond(401, {
        kind: "unauthorized",
        message: "You are signed out of Ranch.",
        loginUrl: "/auth/login",
      })
    fetchMock.mockImplementation(async () => signedOut())
    // Every query on screen fails at once: still one navigation.
    await expect(httpInvoke("list_groups", {})).rejects.toMatchObject({ kind: "unauthorized" })
    await expect(httpInvoke("list_queues", {})).rejects.toMatchObject({ kind: "unauthorized" })
    expect(assign).toHaveBeenCalledTimes(1)
    expect(assign).toHaveBeenCalledWith("/auth/login?return_to=%2Fconnections%2Fc1%3Fview%3Dqueues")
    vi.unstubAllGlobals()
    vi.stubGlobal("fetch", fetchMock)
  })

  it("a network failure is an error with a kind, not an unhandled TypeError", async () => {
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"))
    await expect(httpInvoke("list_groups", {})).rejects.toMatchObject({
      kind: "serverUnreachable",
      message: expect.stringContaining("Failed to fetch"),
    })
  })
})

describe("installHttpBackend", () => {
  it("stands in for Tauri: app commands go over HTTP, plugin calls are inert", async () => {
    installHttpBackend()
    const internals = (
      window as unknown as Record<string, { invoke: (c: string, a?: unknown) => Promise<unknown> }>
    ).__TAURI_INTERNALS__
    fetchMock.mockResolvedValue(respond(200, []))
    await expect(internals?.invoke("list_groups")).resolves.toEqual([])
    expect(fetchMock).toHaveBeenCalledTimes(1)

    // No native window, no event bus: answered locally, nothing on the wire.
    await expect(internals?.invoke("plugin:window|show")).resolves.toBeNull()
    await expect(internals?.invoke("plugin:event|listen", {})).resolves.toEqual(expect.any(Number))
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })
})

describe("runtime", () => {
  it("the server image has no agents and no self-update; the desktop has both", () => {
    setRuntime("server")
    expect(getRuntime()).toBe("server")
    expect(hasAgents()).toBe(false)
    expect(hasUpdater()).toBe(false)
    expect(document.documentElement.dataset.runtime).toBe("server")

    setRuntime("desktop")
    expect(hasAgents()).toBe(true)
    expect(hasUpdater()).toBe(true)

    // The dev preview shows the agent screens and the updater, both on fake data.
    setRuntime("preview")
    expect(hasAgents()).toBe(true)
    expect(hasUpdater()).toBe(true)
  })
})
