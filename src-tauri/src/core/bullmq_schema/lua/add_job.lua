--[[
  Ranch — add a job to a BullMQ queue (ported from BullMQ's own).

  A port of what BullMQ's addStandardJob / addDelayedJob / addPrioritizedJob
  do for a plain job (no parent, no repeat, no deduplication, auto id), in
  one script so the placement is atomic with the paused / concurrency check.
  Checked against real BullMQ 5.x and 6.x workers (tests/add_job_compat_live.rs).

  KEYS[1] wait          KEYS[6]  prioritized
  KEYS[2] paused        KEYS[7]  active
  KEYS[3] meta          KEYS[8]  events (stream)
  KEYS[4] id (counter)  KEYS[9]  marker
  KEYS[5] delayed       KEYS[10] pc (priority counter)

  ARGV[1] job key prefix, e.g. "bull:emails:"
  ARGV[2] name          ARGV[5] timestamp (ms)   ARGV[8] lifo ("1" | "0")
  ARGV[3] data (JSON)   ARGV[6] delay (ms)
  ARGV[4] opts (JSON)   ARGV[7] priority         ARGV[9] custom job id ("" = next)

  Returns { jobId, placement } — placement is "waiting", "paused",
  "delayed" or "prioritized"; "exists" when a custom id is already taken
  (nothing is written, like BullMQ's "duplicated").
]]
local rcall = redis.call
local metaKey = KEYS[3]
local eventsKey = KEYS[8]
local markerKey = KEYS[9]
local timestamp = tonumber(ARGV[5])
local delay = tonumber(ARGV[6])
local priority = tonumber(ARGV[7])

-- getOrSetMaxEvents
local maxEvents = rcall("HGET", metaKey, "opts.maxLenEvents")
if not maxEvents then
  maxEvents = 10000
  rcall("HSET", metaKey, "opts.maxLenEvents", maxEvents)
end

local jobId
local jobIdKey
if ARGV[9] ~= nil and ARGV[9] ~= "" then
  -- A custom id: taken = nothing to do.
  jobId = ARGV[9]
  jobIdKey = ARGV[1] .. jobId
  if rcall("EXISTS", jobIdKey) == 1 then
    rcall("XADD", eventsKey, "*", "event", "duplicated", "jobId", jobId)
    return { jobId, "exists" }
  end
else
  -- Next id. A custom-id job may already sit on that number: never overwrite.
  jobId = rcall("INCR", KEYS[4])
  jobIdKey = ARGV[1] .. jobId
  while rcall("EXISTS", jobIdKey) == 1 do
    jobId = rcall("INCR", KEYS[4])
    jobIdKey = ARGV[1] .. jobId
  end
end

-- storeJob
rcall("HMSET", jobIdKey, "name", ARGV[2], "data", ARGV[3], "opts", ARGV[4],
      "timestamp", ARGV[5], "delay", delay, "priority", priority)
rcall("XADD", eventsKey, "*", "event", "added", "jobId", jobId, "name", ARGV[2])

-- (isPausedOrMaxed, isPaused)
local function queueState()
  local attrs = rcall("HMGET", metaKey, "paused", "concurrency")
  if attrs[1] then
    return true, true
  end
  if attrs[2] then
    return rcall("LLEN", KEYS[7]) >= tonumber(attrs[2]), false
  end
  return false, false
end

local placement

if delay > 0 then
  -- getDelayedScore: 12 low bits keep insertion order within one millisecond.
  local delayedKey = KEYS[5]
  local delayedTimestamp = timestamp + delay
  local minScore = delayedTimestamp * 0x1000
  local maxScore = (delayedTimestamp + 1) * 0x1000 - 1
  local score = minScore
  local result = rcall("ZREVRANGEBYSCORE", delayedKey, maxScore, minScore,
                       "WITHSCORES", "LIMIT", 0, 1)
  local currentMaxScore = tonumber(result[2])
  if currentMaxScore ~= nil then
    if currentMaxScore >= maxScore then
      score = maxScore
    else
      score = currentMaxScore + 1
    end
  end
  rcall("ZADD", delayedKey, score, jobId)
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "delayed",
        "jobId", jobId, "delay", delayedTimestamp)
  -- addDelayMarkerIfNeeded: wake a worker when the earliest delayed job is due.
  local nextDelayed = rcall("ZRANGE", delayedKey, 0, 0, "WITHSCORES")
  local nextScore = tonumber(nextDelayed[2])
  if nextScore ~= nil then
    rcall("ZADD", markerKey, nextScore / 0x1000, "1")
  end
  placement = "delayed"
elseif priority > 0 then
  local isPausedOrMaxed = queueState()
  local prioCounter = rcall("INCR", KEYS[10])
  rcall("ZADD", KEYS[6], priority * 0x100000000 + prioCounter % 0x100000000, jobId)
  if not isPausedOrMaxed then
    rcall("ZADD", markerKey, 0, "0")
  end
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "waiting", "jobId", jobId)
  placement = "prioritized"
else
  local isPausedOrMaxed, isPaused = queueState()
  local target = KEYS[1]
  placement = "waiting"
  if isPaused then
    target = KEYS[2]
    placement = "paused"
  end
  local pushCmd = "LPUSH"
  if ARGV[8] == "1" then
    pushCmd = "RPUSH"
  end
  rcall(pushCmd, target, jobId)
  if not isPausedOrMaxed then
    rcall("ZADD", markerKey, 0, "0")
  end
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "waiting", "jobId", jobId)
end

return { jobId .. "", placement }
