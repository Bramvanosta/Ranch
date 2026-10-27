--[[
  Ranch — helpers shared by the maintenance scripts (ported from BullMQ's own).
  Prepended to each script by `bullmq_schema::maintenance`. Ports of the
  BullMQ includes of the same name.
]]
local rcall = redis.call

local function getOrSetMaxEvents(metaKey)
  local maxEvents = rcall("HGET", metaKey, "opts.maxLenEvents")
  if not maxEvents then
    maxEvents = 10000
    rcall("HSET", metaKey, "opts.maxLenEvents", maxEvents)
  end
  return maxEvents
end

-- getTargetQueueList: (target list, isPausedOrMaxed)
local function getTargetQueueList(metaKey, activeKey, waitKey, pausedKey)
  local attrs = rcall("HMGET", metaKey, "paused", "concurrency")
  if attrs[1] then
    return pausedKey, true
  end
  if attrs[2] then
    return waitKey, rcall("LLEN", activeKey) >= tonumber(attrs[2])
  end
  return waitKey, false
end

-- A job that is part of a flow — a child (has a parent) or a parent that
-- still tracks children. Removing one means rewriting the other side's
-- bookkeeping; Ranch leaves flows alone and reports them as skipped.
local function isFlowMember(jobKey)
  local parentKey = rcall("HGET", jobKey, "parentKey")
  if parentKey and parentKey ~= "" then
    return true
  end
  return rcall("EXISTS", jobKey .. ":dependencies") == 1
end

-- removeJobKeys + removeDeduplicationKeyIfNeededOnRemoval
local function removeJob(prefix, jobId)
  local jobKey = prefix .. jobId
  local deduplicationId = rcall("HGET", jobKey, "deid")
  if deduplicationId then
    local deduplicationKey = prefix .. "de:" .. deduplicationId
    if rcall("GET", deduplicationKey) == jobId then
      rcall("DEL", deduplicationKey, prefix .. "dn:" .. deduplicationId)
    end
  end
  rcall("DEL", jobKey, jobKey .. ":logs", jobKey .. ":dependencies", jobKey .. ":processed",
        jobKey .. ":failed", jobKey .. ":unsuccessful", jobKey .. ":lock")
end

--[[
  reprocessJob, for a failed job: back in line, its failure erased, its flow
  parent (if any) waiting for it again. Returns 1, -1 (no such job) or -3
  (not in the failed set).
  K = { failed, wait, paused, meta, active, events, marker }
]]
local function retryFailedJob(K, prefix, jobId, maxEvents)
  local jobKey = prefix .. jobId
  if rcall("ZREM", K[1], jobId) ~= 1 then
    if rcall("EXISTS", jobKey) == 1 then
      return -3
    end
    return -1
  end
  if rcall("EXISTS", jobKey) ~= 1 then
    -- The set pointed at a job whose hash is gone: nothing to run.
    return -1
  end
  rcall("HDEL", jobKey, "finishedOn", "processedOn", "failedReason")
  local target, isPausedOrMaxed = getTargetQueueList(K[4], K[5], K[2], K[3])
  local pushCmd = "LPUSH"
  local opts = rcall("HGET", jobKey, "opts")
  if opts and string.find(opts, '"lifo":true', 1, true) then
    pushCmd = "RPUSH"
  end
  rcall(pushCmd, target, jobId)
  if not isPausedOrMaxed then
    rcall("ZADD", K[7], 0, "0")
  end
  local parentKey = rcall("HGET", jobKey, "parentKey")
  if parentKey and parentKey ~= "" and rcall("EXISTS", parentKey) == 1 then
    if rcall("ZREM", parentKey .. ":unsuccessful", jobKey) == 1 or
       rcall("HDEL", parentKey .. ":failed", jobKey) == 1 then
      rcall("SADD", parentKey .. ":dependencies", jobKey)
    end
  end
  rcall("XADD", K[6], "MAXLEN", "~", maxEvents, "*", "event", "waiting",
        "jobId", jobId, "prev", "failed")
  return 1
end
