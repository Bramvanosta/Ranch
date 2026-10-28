/** Query keys of what a connection's server says about itself. */
export const serverKeys = {
  info: (connectionId: string) => ["serverInfo", connectionId] as const,
}
