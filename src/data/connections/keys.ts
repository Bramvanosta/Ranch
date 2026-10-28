/** Query keys of saved connections. */
export const connectionKeys = {
  /** Every saved connection: the sidebar, the home table and most screens read it. */
  all: ["connections"] as const,
  ping: (connectionId: string) => ["ping", connectionId] as const,
  /** The kinds of backend a connection may name, and what each one's form asks. */
  backends: ["backends"] as const,
}
