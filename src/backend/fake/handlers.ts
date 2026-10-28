import type { commands } from "@/lib/tauri/bindings"

/**
 * The fake backend answers `invoke(name, args)` the way the Rust commands
 * do. Its handlers are typed against the generated bindings: a handler named
 * after a command must return that command's data, so a change to a result
 * type on the Rust side shows up here at the type check.
 */

type Commands = typeof commands

/** `listConnections` → `list_connections`: the name `invoke` receives. */
type SnakeCase<S extends string> = S extends `${infer Head}${infer Tail}`
  ? Head extends Lowercase<Head>
    ? `${Head}${SnakeCase<Tail>}`
    : `_${Lowercase<Head>}${SnakeCase<Tail>}`
  : S

type CamelCase<S extends string> = S extends `${infer Head}_${infer Tail}`
  ? `${Head}${Capitalize<CamelCase<Tail>>}`
  : S

type DataOf<R> =
  R extends Promise<infer Result>
    ? Result extends { status: "ok"; data: infer Data }
      ? Data
      : never
    : never

/** Every command, by the name `invoke` receives. */
export type CommandName = SnakeCase<keyof Commands & string>

/** What a command answers when it succeeds. */
export type CommandData<N extends CommandName> =
  CamelCase<N> extends keyof Commands ? DataOf<ReturnType<Commands[CamelCase<N>]>> : never

/** The arguments as `invoke` passes them: `{ args }`, or the named parameters. */
export type InvokeArgs = Record<string, unknown>

export type FakeHandlers = { [N in CommandName]?: (args: InvokeArgs) => CommandData<N> }
