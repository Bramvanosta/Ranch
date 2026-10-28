import type { ComponentType } from "react"

import { BULLMQ_REDIS, readRedisEndpoint } from "@/domain/bullmqEndpoint"
import type { Connection, EndpointSchema, TestConnectionArgs } from "@/lib/tauri/bindings"

import { ServerStep, type ServerStepProps } from "./ConnectionServerStep"
import type { ConnectionFormInput, ConnectionFormValues } from "./connectionFormSchema"
import {
  connectionFormDefaults,
  connectionSaveFields,
  STEP_ONE_FIELDS,
  testConnectionArgs,
} from "./connectionFormValues"

/**
 * The connection sheet's form, by backend kind: a connection names its
 * backend, and the sheet asks the form of that kind for its first step
 * (where the server is), the values it opens with, and how they become an
 * endpoint and the secrets sent beside it. Step 2 (the name, the placement,
 * the protections) is the same for every kind.
 */
export interface ConnectionForm {
  /** Step 1: where the server is. */
  ServerStep: ComponentType<ServerStepProps>
  /** The fields step 1 holds, checked before the test. */
  stepOneFields: ReadonlyArray<keyof ConnectionFormInput>
  /** What the form opens with: a new connection, or the one being edited. */
  defaults: (editing: Connection | null, schema?: EndpointSchema) => ConnectionFormInput
  /** The saved endpoint, as step 1 compares what was typed against it. */
  saved: (editing: Connection) => ServerStepProps["saved"]
  /** `test_connection` for what step 1 holds. */
  testArgs: (values: ConnectionFormInput, savedConnectionId: string | null) => TestConnectionArgs
  /** What `add_connection` and `update_connection` share. */
  saveFields: (
    values: ConnectionFormValues,
    locked: boolean | null,
  ) => ReturnType<typeof connectionSaveFields>
}

const bullmqRedisForm: ConnectionForm = {
  ServerStep,
  stepOneFields: STEP_ONE_FIELDS,
  defaults: connectionFormDefaults,
  saved: (editing) => readRedisEndpoint(editing),
  testArgs: testConnectionArgs,
  saveFields: connectionSaveFields,
}

/** The form of each kind of backend Ranch can add or edit. */
export const connectionForms: Readonly<Record<string, ConnectionForm>> = {
  [BULLMQ_REDIS]: bullmqRedisForm,
}

/** The kind a new connection starts as. */
export const DEFAULT_BACKEND = BULLMQ_REDIS
