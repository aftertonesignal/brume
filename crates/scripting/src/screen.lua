-- brume screen module — canvas drawing API for scripts
-- Inspired by monome norns screen API
--
-- Usage:
--   screen.clear()
--   screen.color(0, 204, 102)
--   screen.line(10, 10, 100, 50)
--   screen.text(20, 30, "hello")
--   screen.update()

screen = {}

-- Internal command buffer
screen._cmds = {}
screen._active = false

--- Clear the screen.
function screen.clear()
  screen._active = true
  table.insert(screen._cmds, "c")
end

--- Set drawing color (0-255 per channel).
function screen.color(r, g, b, a)
  a = a or 255
  table.insert(screen._cmds, string.format("s|rgba(%d,%d,%d,%.2f)", r, g, b, a/255))
end

--- Draw a line.
function screen.line(x1, y1, x2, y2)
  table.insert(screen._cmds, string.format("l|%d|%d|%d|%d", x1, y1, x2, y2))
end

--- Draw a rectangle outline.
function screen.rect(x, y, w, h)
  table.insert(screen._cmds, string.format("r|%d|%d|%d|%d", x, y, w, h))
end

--- Draw a filled rectangle.
function screen.fill_rect(x, y, w, h)
  table.insert(screen._cmds, string.format("fr|%d|%d|%d|%d", x, y, w, h))
end

--- Draw a circle outline.
function screen.circle(x, y, r)
  table.insert(screen._cmds, string.format("ci|%d|%d|%d", x, y, r))
end

--- Draw a filled circle.
function screen.fill_circle(x, y, r)
  table.insert(screen._cmds, string.format("fc|%d|%d|%d", x, y, r))
end

--- Draw text at a position.
function screen.text(x, y, str)
  table.insert(screen._cmds, string.format("t|%d|%d|%s", x, y, str))
end

--- Set line width.
function screen.line_width(w)
  table.insert(screen._cmds, string.format("w|%.1f", w))
end

--- Set font size.
function screen.font_size(size)
  table.insert(screen._cmds, string.format("f|%d", size))
end

--- Draw a pixel.
function screen.pixel(x, y)
  table.insert(screen._cmds, string.format("fr|%d|%d|1|1", x, y))
end

--- Move to point (for paths).
function screen.move(x, y)
  table.insert(screen._cmds, string.format("m|%d|%d", x, y))
end

--- Line to point (for paths).
function screen.line_to(x, y)
  table.insert(screen._cmds, string.format("lt|%d|%d", x, y))
end

--- Stroke the current path.
function screen.stroke()
  table.insert(screen._cmds, "st")
end

--- Flush drawing commands to the display.
function screen.update()
  table.insert(screen._cmds, "u")
end

--- Internal: drain command buffer. Returns concatenated commands or nil.
function screen._drain()
  if #screen._cmds == 0 then return nil end
  local result = table.concat(screen._cmds, "\n")
  screen._cmds = {}
  return result
end
