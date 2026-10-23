export function safeEngineLine(line: string): string | undefined {
  if (/\b(DEBUG\d*|VERBOSE)\b/.test(line)) return
  return line
    .replace(/(clipboard invalid size header:).*$/i, '$1 [REDACTED]')
    .replace(
      /((?:clipboard(?:data|content|text)|clipboard\s+(?:content|text))\s*[:=]).*$/i,
      '$1 [REDACTED]',
    )
}
/** Keep useful lifecycle messages; never collect verbose input/clipboard traces. */
export function engineLifecycleLine(line: string) {
  if (
    /\b(DEBUG\d*|VERBOSE)\b|clipboard|keystroke|key(?:board)?\s+(?:down|up|event|press)|mouse\s+(?:move|button|event)/i.test(
      line,
    )
  )
    return false
  return (
    /\bINFO\b/.test(line) &&
    /starting|started|stopped|connecting|connected|disconnected|accepted|client|server|listen|switch from|version|configuration|TLS|fingerprint|certificate/i.test(
      line,
    )
  )
}
