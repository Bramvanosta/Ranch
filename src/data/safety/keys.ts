/** Query keys of the session lock. */
export const safetyKeys = {
  lockState: (connectionId: string) => ["lockState", connectionId] as const,
}
