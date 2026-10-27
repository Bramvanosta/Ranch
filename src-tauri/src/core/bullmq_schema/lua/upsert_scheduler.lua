--[[
  Ranch — create or replace a job scheduler (BullMQ `addJobScheduler`).

  A scheduler = its entry in `repeat` (score = next run), its hash
  `repeat:{id}`, and ONE pending job `repeat:{id}:{millis}`. Workers create
  the following job when they finish that one; nothing else does.

  KEYS[1] repeat   KEYS[5] meta         KEYS[9]  events
  KEYS[2] delayed  KEYS[6] prioritized  KEYS[10] pc
  KEYS[3] wait     KEYS[7] marker       KEYS[11] active
  KEYS[4] paused   KEYS[8] id

  ARGV[1] job key prefix    ARGV[6] every (ms) or ""
  ARGV[2] scheduler id      ARGV[7] first run (ms) — pattern only, else ""
  ARGV[3] job name          ARGV[8] now (ms)
  ARGV[4] pattern or ""     ARGV[9] data template (JSON)
  ARGV[5] tz or ""

  Returns { jobId, delay } — or { "", code }: -10 a job already exists for
  that run of the pattern, -11 every slot tried for `every` is taken.
]]
local repeatKey, delayedKey, waitKey, pausedKey = KEYS[1], KEYS[2], KEYS[3], KEYS[4]
local metaKey, prioritizedKey, markerKey, eventsKey = KEYS[5], KEYS[6], KEYS[7], KEYS[9]
local prefix = ARGV[1]
local id = ARGV[2]
local name = ARGV[3]
local pattern = ARGV[4] ~= "" and ARGV[4] or nil
local tz = ARGV[5] ~= "" and ARGV[5] or nil
local every = tonumber(ARGV[6])
local now = tonumber(ARGV[8])
local templateData = ARGV[9]
local schedulerKey = repeatKey .. ":" .. id
local maxEvents = getOrSetMaxEvents(metaKey)

-- getJobSchedulerEveryNextMillis (no startDate, no caller offset)
local function everyNextMillis(prevMillis)
  local nextMillis
  if not prevMillis then
    nextMillis = now
  else
    nextMillis = prevMillis + every
    if nextMillis < now then
      local aligned = math.floor(now / every) * every
      if aligned <= now then
        nextMillis = aligned + every
      else
        nextMillis = aligned
      end
    end
  end
  local offset = nextMillis - math.floor(nextMillis / every) * every
  return math.floor(nextMillis), math.floor(offset)
end

local prevMillis = tonumber(rcall("ZSCORE", repeatKey, id))
local nextMillis
local newOffset = 0
local updatedEvery = false
if every then
  local millis = prevMillis
  if prevMillis and tonumber(rcall("HGET", schedulerKey, "every")) ~= every then
    -- The interval changed: start a fresh cadence.
    millis = nil
    updatedEvery = true
  end
  nextMillis, newOffset = everyNextMillis(millis)
else
  nextMillis = tonumber(ARGV[7])
  if nextMillis < now then
    nextMillis = now
  end
end

-- Replace the pending job of the previous version of this scheduler.
local removedPrevJob = false
if prevMillis then
  local currentJobId = "repeat:" .. id .. ":" .. prevMillis
  if rcall("EXISTS", prefix .. currentJobId) == 1 then
    if rcall("ZREM", delayedKey, currentJobId) == 1 or
       rcall("ZREM", prioritizedKey, currentJobId) == 1 or
       rcall("LREM", waitKey, 1, currentJobId) > 0 or
       rcall("LREM", pausedKey, 1, currentJobId) > 0 then
      removeJob(prefix, currentJobId)
      removedPrevJob = true
    end
    -- else: it is active (or finished) — a worker owns it; leave it.
  end
end

local storeOffset = nil
if removedPrevJob then
  if every and not updatedEvery then
    nextMillis = prevMillis
  end
else
  storeOffset = newOffset
end

-- A job may already sit on that run (finished, or active right now).
local jobId = "repeat:" .. id .. ":" .. nextMillis
if rcall("EXISTS", prefix .. jobId) == 1 then
  if every then
    local scanned = 0
    repeat
      nextMillis = nextMillis + every
      jobId = "repeat:" .. id .. ":" .. nextMillis
      scanned = scanned + 1
    until rcall("EXISTS", prefix .. jobId) == 0 or scanned >= 32
    if rcall("EXISTS", prefix .. jobId) == 1 then
      return { "", -11 }
    end
  elseif not removedPrevJob then
    return { "", -10 }
  end
end
local delay = nextMillis - now
if delay < 0 then
  delay = 0
end

-- storeJobScheduler
local fields = {}
if tz then fields[#fields + 1] = "tz"; fields[#fields + 1] = tz end
if pattern then fields[#fields + 1] = "pattern"; fields[#fields + 1] = pattern end
if every then fields[#fields + 1] = "every"; fields[#fields + 1] = every end
local offset = storeOffset
if offset == nil then
  offset = tonumber(rcall("HGET", schedulerKey, "offset"))
end
if offset ~= nil then fields[#fields + 1] = "offset"; fields[#fields + 1] = offset end
if templateData ~= "{}" then fields[#fields + 1] = "data"; fields[#fields + 1] = templateData end
local ic = tonumber(rcall("HGET", schedulerKey, "ic")) or 1
fields[#fields + 1] = "ic"; fields[#fields + 1] = ic
rcall("ZADD", repeatKey, nextMillis, id)
rcall("DEL", schedulerKey)
rcall("HMSET", schedulerKey, "name", name, unpack(fields))

-- addJobFromScheduler: the pending job, carrying what a worker needs to
-- compute the run after it (`opts.repeat`, `prevMillis`).
rcall("INCR", KEYS[8])
local repeatOpts = { count = ic }
if pattern then repeatOpts["pattern"] = pattern end
if tz then repeatOpts["tz"] = tz end
if every then repeatOpts["every"] = every end
local jobOpts = {
  ["repeat"] = repeatOpts,
  repeatJobKey = id,
  jobId = jobId,
  timestamp = now,
  delay = delay,
}
if pattern then jobOpts["prevMillis"] = nextMillis end

local jobKey = prefix .. jobId
rcall("HMSET", jobKey, "name", name, "data", templateData, "opts", cjson.encode(jobOpts),
      "timestamp", ARGV[8], "delay", delay, "priority", 0, "rjk", id)
rcall("XADD", eventsKey, "*", "event", "added", "jobId", jobId, "name", name)

if delay ~= 0 then
  -- addDelayedJob
  local minScore = nextMillis * 0x1000
  local maxScore = (nextMillis + 1) * 0x1000 - 1
  local score = minScore
  local result = rcall("ZREVRANGEBYSCORE", delayedKey, maxScore, minScore, "WITHSCORES", "LIMIT", 0, 1)
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
        "jobId", jobId, "delay", nextMillis)
  local nextDelayed = rcall("ZRANGE", delayedKey, 0, 0, "WITHSCORES")
  local nextScore = tonumber(nextDelayed[2])
  if nextScore ~= nil then
    rcall("ZADD", markerKey, nextScore / 0x1000, "1")
  end
else
  local target, isPausedOrMaxed = getTargetQueueList(metaKey, KEYS[11], waitKey, pausedKey)
  rcall("LPUSH", target, jobId)
  if not isPausedOrMaxed then
    rcall("ZADD", markerKey, 0, "0")
  end
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "waiting", "jobId", jobId)
end

return { jobId, delay }
