/**
 * What the payload editor says as you type: "Valid JSON ·
 * 4 keys · 98 bytes", or what is wrong and on which line.
 *
 * A small recursive-descent reader rather than `JSON.parse`: the engines'
 * messages differ and some do not say where the error is. It only checks;
 * the value itself still comes from `JSON.parse`.
 */
export type JsonCheck =
  | { ok: true; summary: string }
  | { ok: false; message: string; line: number; column: number }

export function checkJson(text: string): JsonCheck {
  if (text.trim() === "") {
    return {
      ok: false,
      message: "The payload is required: use {} for an empty one",
      ...at(text, 0),
    }
  }
  const reader = new Reader(text)
  try {
    reader.skipSpace()
    reader.value()
    reader.skipSpace()
    if (reader.pos < text.length) reader.fail("Unexpected text after the value")
  } catch (e) {
    if (e instanceof JsonSyntaxError) return { ok: false, message: e.message, ...at(text, e.pos) }
    throw e
  }
  const value: unknown = JSON.parse(text)
  // What Redis will hold: the payload without the spaces between tokens.
  const bytes = new TextEncoder().encode(minifyJson(text)).length
  return { ok: true, summary: `Valid JSON · ${describe(value)} · ${bytes.toLocaleString()} bytes` }
}

/**
 * Drop the whitespace between the tokens of valid JSON, keeping key order
 * and number spelling: what the backend stores (`add_job::minify_json`).
 */
export function minifyJson(valid: string): string {
  let out = ""
  let inString = false
  let escaped = false
  for (const c of valid) {
    if (inString) {
      out += c
      if (escaped) escaped = false
      else if (c === "\\") escaped = true
      else if (c === '"') inString = false
    } else if (c === '"') {
      inString = true
      out += c
    } else if (!" \t\n\r".includes(c)) {
      out += c
    }
  }
  return out
}

function describe(value: unknown): string {
  if (Array.isArray(value)) return value.length === 1 ? "1 item" : `${value.length} items`
  if (value !== null && typeof value === "object") {
    const n = Object.keys(value).length
    return n === 1 ? "1 key" : `${n} keys`
  }
  return value === null ? "null" : typeof value
}

function at(text: string, pos: number): { line: number; column: number } {
  const before = text.slice(0, pos)
  const line = before.split("\n").length
  const column = pos - before.lastIndexOf("\n")
  return { line, column }
}

class JsonSyntaxError extends Error {
  constructor(
    message: string,
    readonly pos: number,
  ) {
    super(message)
  }
}

class Reader {
  pos = 0
  constructor(private readonly s: string) {}

  fail(message: string): never {
    throw new JsonSyntaxError(message, this.pos)
  }

  skipSpace() {
    while (this.pos < this.s.length && " \t\n\r".includes(this.s[this.pos] ?? "")) this.pos++
  }

  value(): void {
    const c = this.s[this.pos]
    if (c === "{") {
      this.object()
      return
    }
    if (c === "[") {
      this.array()
      return
    }
    if (c === '"') {
      this.string()
      return
    }
    if (c === "-" || (c !== undefined && c >= "0" && c <= "9")) {
      this.number()
      return
    }
    for (const word of ["true", "false", "null"]) {
      if (this.s.startsWith(word, this.pos)) {
        this.pos += word.length
        return
      }
    }
    if (c === undefined) this.fail("The JSON ends too early: a value is missing")
    if (c === "'") this.fail("Strings take double quotes in JSON")
    this.fail(`Expected a value, found ${quote(c)}`)
  }

  object() {
    this.pos++ // {
    this.skipSpace()
    if (this.s[this.pos] === "}") {
      this.pos++
      return
    }
    for (;;) {
      this.skipSpace()
      if (this.s[this.pos] !== '"') {
        if (this.s[this.pos] === "}") this.fail("No comma after the last property in JSON")
        this.fail("Expected a property name in double quotes")
      }
      this.string()
      this.skipSpace()
      if (this.s[this.pos] !== ":") this.fail("Expected ':' after the property name")
      this.pos++
      this.skipSpace()
      this.value()
      this.skipSpace()
      const c = this.s[this.pos]
      if (c === ",") {
        this.pos++
        continue
      }
      if (c === "}") {
        this.pos++
        return
      }
      if (c === undefined) this.fail("The object is not closed: '}' is missing")
      this.fail("Expected ',' or '}' after the value")
    }
  }

  array() {
    this.pos++ // [
    this.skipSpace()
    if (this.s[this.pos] === "]") {
      this.pos++
      return
    }
    for (;;) {
      this.skipSpace()
      if (this.s[this.pos] === "]") this.fail("No comma after the last item in JSON")
      this.value()
      this.skipSpace()
      const c = this.s[this.pos]
      if (c === ",") {
        this.pos++
        continue
      }
      if (c === "]") {
        this.pos++
        return
      }
      if (c === undefined) this.fail("The array is not closed: ']' is missing")
      this.fail("Expected ',' or ']' after the item")
    }
  }

  string() {
    const start = this.pos
    this.pos++ // "
    while (this.pos < this.s.length) {
      const c = this.s[this.pos]
      if (c === '"') {
        this.pos++
        return
      }
      if (c === "\n") this.fail('A string cannot span lines: close it with "')
      if (c === "\\") {
        const next = this.s[this.pos + 1]
        if (next === "u") {
          if (!/^[0-9a-fA-F]{4}$/.test(this.s.slice(this.pos + 2, this.pos + 6))) {
            this.fail("\\u takes four hex digits")
          }
          this.pos += 6
          continue
        }
        if (next === undefined || !'"\\/bfnrt'.includes(next)) this.fail("Unknown escape")
        this.pos += 2
        continue
      }
      this.pos++
    }
    this.pos = start
    this.fail("The string is not closed: '\"' is missing")
  }

  number() {
    const m = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?/.exec(this.s.slice(this.pos))
    if (!m || m[0] === "-") this.fail("Not a valid number")
    this.pos += m[0].length
  }
}

function quote(c: string): string {
  return c === "\n" ? "a line break" : `'${c}'`
}
