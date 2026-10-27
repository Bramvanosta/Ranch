--[[
  Ranch — remove a job scheduler and its pending job (BullMQ
  `removeJobScheduler`), or — ARGV[3] = "pause" — only its pending job.

  KEYS[1] repeat  KEYS[2] delayed  KEYS[3] events  KEYS[4] wait
  KEYS[5] paused  KEYS[6] prioritized
  ARGV[1] scheduler id   ARGV[2] job key prefix   ARGV[3] "remove" | "pause"

  Upstream only looks in `delayed`; the pending job of an `every` scheduler
  that is due now sits in `wait`, so Ranch looks there too. An active job is
  a worker's: it is left alone, and creates no successor once the scheduler
  is gone (BullMQ checks `ZSCORE repeat id` before iterating).

  Pause (a Ranch notion — BullMQ has none): the scheduler stays listed,
  with `paused = 1`; without a pending job nothing fires. Resuming is an
  upsert with the stored settings.

  Returns { code, removedPendingJob }: code 0 ok, 1 no such scheduler.
]]
local id = ARGV[1]
local prefix = ARGV[2]
local millis = rcall("ZSCORE", KEYS[1], id)
if not millis then
  return { 1, 0 }
end
local removedPending = 0
local jobId = "repeat:" .. id .. ":" .. millis
if rcall("ZREM", KEYS[2], jobId) == 1 then
  removedPending = 1
  rcall("XADD", KEYS[3], "*", "event", "removed", "jobId", jobId, "prev", "delayed")
elseif rcall("ZREM", KEYS[6], jobId) == 1 or
       rcall("LREM", KEYS[4], 1, jobId) > 0 or
       rcall("LREM", KEYS[5], 1, jobId) > 0 then
  removedPending = 1
  rcall("XADD", KEYS[3], "*", "event", "removed", "jobId", jobId, "prev", "waiting")
end
if removedPending == 1 then
  removeJob(prefix, jobId)
end

local schedulerKey = KEYS[1] .. ":" .. id
if ARGV[3] == "pause" then
  rcall("HSET", schedulerKey, "paused", "1")
else
  rcall("ZREM", KEYS[1], id)
  rcall("DEL", schedulerKey)
end
return { 0, removedPending }
