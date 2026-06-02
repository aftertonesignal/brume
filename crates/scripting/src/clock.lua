-- brume clock module — coroutine-based musical timing
-- Inspired by monome norns clock system
--
-- Usage:
--   clock.run(function()
--     while true do
--       brume.note_on(brume.FM, 60, 0.5)
--       clock.sync(1/4)  -- wait a quarter note
--       brume.note_off(brume.FM, 60)
--       clock.sync(1/4)
--     end
--   end)

clock = {}

-- Internal state
clock._threads = {}
clock._next_id = 1

--- Run a function as a clock coroutine.
--- Returns a thread ID that can be used to cancel it.
function clock.run(fn)
  local id = clock._next_id
  clock._next_id = clock._next_id + 1

  local co = coroutine.create(fn)
  clock._threads[id] = {
    co = co,
    wake_beat = 0,  -- resume immediately on next tick
    wake_time = 0,
  }

  return id
end

--- Sleep for a number of beats (musical time).
--- Must be called from inside a clock.run coroutine.
function clock.sync(beats)
  local current = brume.beat()
  local target = current + (beats or 1)
  coroutine.yield({ type = "sync", target = target })
end

--- Sleep for a number of seconds (wall clock time).
--- Must be called from inside a clock.run coroutine.
function clock.sleep(seconds)
  local target = _clock_time() + seconds
  coroutine.yield({ type = "sleep", target = target })
end

--- Cancel a running clock thread.
function clock.cancel(id)
  clock._threads[id] = nil
end

--- Cancel all running clock threads.
function clock.cancel_all()
  clock._threads = {}
end

--- Get the current tempo in BPM.
function clock.get_tempo()
  return brume.bpm()
end

--- Set the tempo in BPM.
function clock.set_tempo(bpm)
  brume.set_tempo(bpm)
end

--- Internal: called by the engine on each tick.
--- Resumes any coroutines whose wait condition is met.
function clock._tick(beat)
  local time = _clock_time()
  for id, thread in pairs(clock._threads) do
    local status = coroutine.status(thread.co)
    if status == "dead" then
      clock._threads[id] = nil
    elseif status == "suspended" then
      local should_resume = false

      if thread.wake_type == "sync" then
        should_resume = beat >= thread.wake_beat
      elseif thread.wake_type == "sleep" then
        should_resume = time >= thread.wake_time
      else
        -- First run or unknown type: resume immediately
        should_resume = true
      end

      if should_resume then
        local ok, result = coroutine.resume(thread.co)
        if not ok then
          print("clock error: " .. tostring(result))
          clock._threads[id] = nil
        elseif result then
          -- Coroutine yielded with a wait request
          if result.type == "sync" then
            thread.wake_type = "sync"
            thread.wake_beat = result.target
          elseif result.type == "sleep" then
            thread.wake_type = "sleep"
            thread.wake_time = result.target
          end
        else
          -- Coroutine returned nil (function ended)
          clock._threads[id] = nil
        end
      end
    end
  end
end
