// Minimal JavaScript tokenizer for display. Not a parser; it only needs to be
// right often enough that code looks like code. Returns [{ cls, text }].

const KEYWORDS = new Set(
  'async await function return const let var if else for of in while do new throw try catch finally typeof instanceof class extends this break continue switch case default yield delete void static get set'.split(
    ' '
  )
)
const LITERALS = new Set(['true', 'false', 'null', 'undefined', 'NaN', 'Infinity'])

const TOKEN =
  /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|('(?:\\.|[^'\\\n])*'|"(?:\\.|[^"\\\n])*"|`(?:\\.|[^`\\])*`)|(\b0x[\da-fA-F_]+\b|\b\d[\d_]*(?:\.\d+)?(?:e[+-]?\d+)?\b)|([A-Za-z_$][\w$]*)|(=>|\?\?=?|\?\.|[=!]==?|[+\-*/%<>&|^]=?|&&|\|\||\+\+|--|[?:!~])|([{}()[\].,;])|(\s+)|(.)/g

export function tokenize(src) {
  const out = []
  let m
  let prev = null // previous non-space token, for ".prop" and "name(" detection
  TOKEN.lastIndex = 0
  while ((m = TOKEN.exec(src))) {
    const [text, comment, string, number, ident, op, punct, space] = m
    let cls = 'tk-plain'
    if (comment) cls = 'tk-comment'
    else if (string) cls = 'tk-string'
    else if (number) cls = 'tk-number'
    else if (ident) {
      if (KEYWORDS.has(ident)) cls = 'tk-kw'
      else if (LITERALS.has(ident)) cls = 'tk-lit'
      else if (prev?.text === '.') cls = 'tk-prop'
      else if (/^\s*\(/.test(src.slice(TOKEN.lastIndex))) cls = 'tk-fn'
      else if (prev?.cls === 'tk-kw' && /^(function|class)$/.test(prev.text)) cls = 'tk-fn'
      else cls = 'tk-ident'
    } else if (op) cls = 'tk-op'
    else if (punct) cls = 'tk-punct'
    else if (space) cls = 'tk-space'
    const tok = { cls, text }
    out.push(tok)
    if (!space) prev = tok
  }
  return out
}
