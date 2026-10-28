/** What the gate will apply: the connection's own flag, else the global default. */
export function effectiveMcpReadonly(
  connection: { mcpReadonly: boolean | null },
  readonlyDefault: boolean,
): boolean {
  return connection.mcpReadonly ?? readonlyDefault
}
