#!/usr/bin/env node
// Keeps STABILITY.md (稳定性分析) in step with the code: finds every place
// that can panic, throw or end the program, or that drops an error without a
// word, and checks that each one is quoted in the document, and that every
// quote is still the code as it is.
//
//   node scripts/stability.mjs              check (CI runs this; exit 1 when out of step)
//   node scripts/stability.mjs --list       every place found, file:line [kind] code
//   node scripts/stability.mjs --skeleton   entries to fill in for places not in the document yet
//   node scripts/stability.mjs --summary    the 「要改的地方」 table, from the entries' verdicts
//   options: --doc <file> (another document), --partial (only the files it has sections for)
//
// Needs frontend/node_modules (npm ci in frontend/): TypeScript and Vue's
// SFC parser read the frontend and the Worker exactly as the compilers do.

import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join, relative } from 'node:path'

const ROOT = new URL('..', import.meta.url).pathname
const require = createRequire(join(ROOT, 'frontend/package.json'))
const ts = require('typescript')
const { parse: parseSfc } = require('vue/compiler-sfc')

const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const docPath = args.includes('--doc') ? args[args.indexOf('--doc') + 1] : join(ROOT, 'STABILITY.md')

// ---- Finding the places ----

function files(dir, ext) {
  const out = []
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name)
      if (e.isDirectory()) walk(p)
      else if (ext.some((x) => e.name.endsWith(x)) && !e.name.endsWith('.d.ts')) out.push(p)
    }
  }
  walk(join(ROOT, dir))
  return out.sort()
}

/** Comments and the inside of string / char literals as spaces (same length, newlines kept). */
function blankRust(src) {
  const out = src.split('')
  const blank = (from, to) => {
    for (let i = from; i < to; i++) if (out[i] !== '\n') out[i] = ' '
  }
  let i = 0
  while (i < src.length) {
    const c = src[i]
    const next = src[i + 1]
    if (c === '/' && next === '/') {
      const end = src.indexOf('\n', i)
      const stop = end < 0 ? src.length : end
      blank(i, stop)
      i = stop
    } else if (c === '/' && next === '*') {
      let depth = 1
      let j = i + 2
      while (j < src.length && depth > 0) {
        if (src[j] === '/' && src[j + 1] === '*') (depth++, (j += 2))
        else if (src[j] === '*' && src[j + 1] === '/') (depth--, (j += 2))
        else j++
      }
      blank(i, j)
      i = j
    } else if (/[rb]/.test(c) && !/\w/.test(src[i - 1] ?? '') && /^(?:b?r#*"|b")/.test(src.slice(i, i + 40))) {
      const m = /^(b?r)(#*)"/.exec(src.slice(i, i + 40))
      if (m) {
        // Raw string: ends at the quote followed by as many #.
        const close = `"${m[2]}`
        const start = i + m[0].length
        const end = src.indexOf(close, start)
        const stop = end < 0 ? src.length : end + close.length
        blank(i + 1, stop - 1)
        out[i] = '"'
        out[stop - 1] = '"'
        i = stop
      } else {
        i = skipString(i + 1)
      }
    } else if (c === '"') {
      i = skipString(i)
    } else if (c === "'") {
      // A char literal, or a lifetime / label ('a, 'static).
      if (next === '\\') {
        const end = src.indexOf("'", i + 2 + (src[i + 2] === "'" ? 1 : 0))
        blank(i + 1, end)
        i = end + 1
      } else {
        const cp = src.codePointAt(i + 1)
        const w = cp > 0xffff ? 2 : 1
        if (src[i + 1 + w] === "'") {
          blank(i + 1, i + 1 + w)
          i += 2 + w
        } else i++
      }
    } else i++
  }
  return out.join('')

  function skipString(start) {
    let j = start + 1
    while (j < src.length && src[j] !== '"') j += src[j] === '\\' ? 2 : 1
    blank(start + 1, j)
    return j + 1
  }
}

/** Drops `#[cfg(test)]` items (test modules and helpers): they are meant to stop at the first surprise. */
function withoutTests(code) {
  const out = code.split('')
  let from = 0
  for (;;) {
    const at = code.indexOf('#[cfg(test)]', from)
    if (at < 0) break
    let i = at + '#[cfg(test)]'.length
    let depth = 0
    while (i < code.length) {
      const c = code[i]
      if (c === '(' || c === '[') depth++
      else if (c === ')' || c === ']') depth--
      else if (depth === 0 && c === ';') {
        i++
        break
      } else if (depth === 0 && c === '{') {
        let braces = 0
        for (; i < code.length; i++) {
          if (code[i] === '{') braces++
          else if (code[i] === '}' && --braces === 0) break
        }
        i++
        break
      }
      i++
    }
    for (let j = at; j < i && j < code.length; j++) if (out[j] !== '\n') out[j] = ' '
    from = i
  }
  return out.join('')
}

const RUST_RULES = [
  [/\.(unwrap|expect|unwrap_err|expect_err)\s*\(/g, (m) => m[1]],
  [/\b(panic|unreachable|todo|unimplemented|assert|assert_eq|assert_ne|debug_assert|debug_assert_eq|debug_assert_ne)!/g, (m) => `${m[1]}!`],
  [/\bunsafe\b/g, () => 'unsafe'],
  [/\bprocess::(exit|abort)\b/g, (m) => `process::${m[1]}`],
  // Panics on an index past the end, or (strings) not on a char boundary.
  [/\.(split_at|split_at_mut|split_off|insert_str|replace_range|truncate|drain)\s*\(/g, (m) => m[1]],
  [/\.insert\s*\([^,()]+,\s*'/g, () => 'insert'],
  [/\.(chunks|chunks_exact|windows)\s*\(\s*(?!\d+\s*\))/g, (m) => m[1]],
  // A result thrown away: an error here is never seen.
  [/\blet _ =/g, () => 'let _ ='],
]

function rustSites(file) {
  const code = withoutTests(blankRust(readFileSync(file, 'utf8')))
  const lineOf = lineIndex(code)
  const sites = []
  for (const [re, kind] of RUST_RULES) {
    for (const m of code.matchAll(re)) sites.push({ line: lineOf(m.index), kind: kind(m) })
  }
  // Indexing and slicing: `x[i]`, `s[a..b]`; not `x[..]` (all of it), and
  // not `v["key"]` (serde_json: a missing key reads as null).
  for (let i = 1; i < code.length; i++) {
    if (code[i] !== '[' || !/[\w)\]?]/.test(code[i - 1])) continue
    let depth = 0
    let j = i
    for (; j < code.length; j++) {
      if (code[j] === '[') depth++
      else if (code[j] === ']' && --depth === 0) break
    }
    const inside = code.slice(i + 1, j).trim()
    if (inside === '..' || /^"\s*"$/.test(inside)) continue
    sites.push({ line: lineOf(i), kind: inside.includes('..') ? 'slice' : 'index' })
  }
  return sites
}

function lineIndex(text) {
  const starts = [0]
  for (let i = 0; i < text.length; i++) if (text[i] === '\n') starts.push(i + 1)
  return (pos) => {
    let lo = 0
    let hi = starts.length - 1
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1
      if (starts[mid] <= pos) lo = mid
      else hi = mid - 1
    }
    return lo + 1
  }
}

const THROWING_CALLS = new Set(['JSON.parse', 'decodeURIComponent', 'decodeURI', 'atob', 'btoa'])

/** Places in TypeScript / JavaScript source, `offset` lines down in its file. */
function scriptSites(text, kind, offset) {
  const sf = ts.createSourceFile('x', text, ts.ScriptTarget.Latest, true, kind)
  const sites = []
  const at = (node, k) => sites.push({ line: offset + sf.getLineAndCharacterOfPosition(node.getStart(sf)).line, kind: k })
  const visit = (node) => {
    if (ts.isThrowStatement(node)) at(node, 'throw')
    // An error dropped without a word: an empty catch, or a .catch that does nothing with it.
    else if (ts.isCatchClause(node) && !node.block.statements.length) at(node, 'catch {}')
    else if (ts.isNonNullExpression(node)) at(node, 'non-null !')
    else if (ts.isCallExpression(node)) {
      const callee = node.expression.getText(sf)
      const f = node.arguments[0]
      if (THROWING_CALLS.has(callee)) at(node, callee)
      else if (
        ts.isPropertyAccessExpression(node.expression) &&
        node.expression.name.text === 'catch' &&
        f &&
        (ts.isArrowFunction(f) || ts.isFunctionExpression(f)) &&
        (ts.isBlock(f.body) ? !f.body.statements.length : !ts.isCallExpression(f.body))
      )
        at(node.expression.name, '.catch 忽略')
      else if (ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'json' && !node.arguments.length)
        at(node, '.json()')
    } else if (ts.isNewExpression(node)) {
      const name = node.expression.getText(sf)
      if (name === 'URL') at(node, 'new URL')
      if (name === 'RegExp' && !(node.arguments?.[0] && ts.isStringLiteral(node.arguments[0]))) at(node, 'new RegExp')
    } else if (ts.isPropertyAccessExpression(node) && /^(window\.)?(localStorage|sessionStorage)$/.test(node.expression.getText(sf))) {
      at(node, 'localStorage')
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return sites
}

function vueSites(file) {
  const { descriptor } = parseSfc(readFileSync(file, 'utf8'), { filename: file })
  const sites = []
  for (const block of [descriptor.script, descriptor.scriptSetup]) {
    if (block) sites.push(...scriptSites(block.content, ts.ScriptKind.TS, block.loc.start.line))
  }
  // Template expressions ({{ }}, :prop, @event, v-if ...).
  const walk = (node) => {
    const exps = []
    if (node.type === 5) exps.push(node.content)
    for (const p of node.props ?? []) if (p.type === 7) exps.push(p.exp, p.arg)
    for (const e of exps) {
      if (e?.type === 4 && !e.isStatic) sites.push(...scriptSites(e.content, ts.ScriptKind.TS, e.loc.start.line))
    }
    for (const c of node.children ?? []) walk(c)
  }
  if (descriptor.template?.ast) walk(descriptor.template.ast)
  return sites
}

function allSites() {
  const byFile = new Map()
  const add = (file, sites) => {
    if (!sites.length) return
    const rel = relative(ROOT, file)
    const lines = readFileSync(file, 'utf8').split('\n')
    const seen = new Set()
    byFile.set(
      rel,
      sites
        .filter((s) => {
          const k = `${s.line}|${s.kind}`
          return !seen.has(k) && seen.add(k)
        })
        .sort((a, b) => a.line - b.line)
        .map((s) => ({ ...s, file: rel, text: lines[s.line - 1].trim() })),
    )
  }
  for (const f of files('backend/src', ['.rs'])) add(f, rustSites(f))
  for (const f of files('frontend/src', ['.ts'])) add(f, scriptSites(readFileSync(f, 'utf8'), ts.ScriptKind.TS, 1))
  for (const f of files('frontend/src', ['.vue'])) add(f, vueSites(f))
  for (const f of files('worker/src', ['.js'])) add(f, scriptSites(readFileSync(f, 'utf8'), ts.ScriptKind.JS, 1))
  return byFile
}

// ---- The document ----

const ELISION = /^(\/\/\s*…|<!--\s*…\s*-->|…)$/
const VERDICTS = ['不用改', '可以改（低）', '建议改（中）', '建议改（高）']
const SUMMARY = '## 要改的地方'

/**
 * File sections (`## \`path\``), their entries (`### ...`) with each one's
 * quoted code and verdict, and the rows of the 「要改的地方」 table.
 */
function readDoc(path) {
  const sections = new Map()
  const summary = []
  let file = null
  let entry = null
  let fence = null
  let inSummary = false
  for (const [n, raw] of readFileSync(path, 'utf8').split('\n').entries()) {
    if (fence) {
      if (/^```\s*$/.test(raw)) {
        entry?.snippets.push(fence)
        fence = null
      } else fence.lines.push(raw.trim())
      continue
    }
    const h2 = /^## `([^`]+)`\s*$/.exec(raw)
    if (h2) {
      file = h2[1]
      entry = null
      if (!sections.has(file)) sections.set(file, [])
      continue
    }
    if (/^## /.test(raw)) {
      file = entry = null
      inSummary = raw.trim() === SUMMARY
    }
    const row = inSummary && /^\|\s*(高|中)\s*\|\s*`([^`]+)`\s*\|\s*([^|]+?)\s*\|/.exec(raw)
    if (row) summary.push({ level: row[1], file: row[2], title: row[3] })
    const verdict = entry && /^- \*\*要不要改\*\*：\s*(\S+?（[低中高]）|不用改)/.exec(raw)
    if (verdict) entry.verdict = verdict[1]
    const h3 = /^### (.+)$/.exec(raw)
    if (h3 && file) {
      entry = { title: h3[1].trim(), line: n + 1, snippets: [], manual: h3[1].includes('（人工）') }
      sections.get(file).push(entry)
      continue
    }
    if (/^```/.test(raw) && file) fence = { lines: [], line: n + 1 }
  }
  return { sections, summary }
}

/** Where a quote is in its file: the source line numbers it shows, or why not. */
function locate(snippet, source) {
  const segments = [[]]
  for (const l of snippet.lines) {
    if (!l) continue
    if (ELISION.test(l)) segments.push([])
    else segments.at(-1).push(l)
  }
  const segs = segments.filter((s) => s.length)
  if (!segs.length) return { error: '空的代码块' }
  const find = (seg, from) => {
    for (let p = from; p + seg.length <= source.length; p++) {
      if (seg.every((l, k) => source[p + k].text === l)) return p
    }
    return -1
  }
  const matches = []
  for (let start = find(segs[0], 0); start >= 0; start = find(segs[0], start + 1)) {
    const lines = []
    let p = start
    let ok = true
    for (const [k, seg] of segs.entries()) {
      const q = k === 0 ? start : find(seg, p)
      if (q < 0) {
        ok = false
        break
      }
      for (let r = 0; r < seg.length; r++) lines.push(source[q + r].line)
      p = q + seg.length
    }
    if (ok) matches.push(lines)
  }
  if (!matches.length) return { error: '代码里找不到这段（代码改了？）' }
  if (matches.length > 1) return { error: `代码里有 ${matches.length} 处一样的，多引几行让它唯一` }
  return { lines: new Set(matches[0]) }
}

/** A quote for a place: the line and enough before it to be the only such run in the file. */
function suggest(site, source) {
  const idx = source.findIndex((s) => s.line === site.line)
  for (let before = 0; before <= 6; before++) {
    const seg = source.slice(Math.max(0, idx - before), idx + 1)
    const text = seg.map((s) => s.text)
    const count = source.filter((_, p) => text.every((l, k) => source[p + k]?.text === l)).length
    if (count === 1 || before === 6) return text
  }
}

// ---- Run ----

const sites = allSites()
const lang = (f) => (f.endsWith('.rs') ? 'rust' : f.endsWith('.vue') ? 'vue' : f.endsWith('.js') ? 'js' : 'ts')

if (flag('--list')) {
  for (const list of sites.values()) for (const s of list) console.log(`${s.file}:${s.line} [${s.kind}] ${s.text}`)
  process.exit(0)
}

const { sections: doc, summary } = existsSync(docPath) ? readDoc(docPath) : { sections: new Map(), summary: [] }
const problems = []
const advised = [...doc].flatMap(([file, entries]) =>
  entries.filter((e) => e.verdict?.startsWith('建议改')).map((e) => ({ file, title: e.title, level: e.verdict.slice(-2, -1) })),
)
if (flag('--summary')) {
  console.log(`| 级别 | 文件 | 条目 | 怎么改 |\n|---|---|---|---|`)
  for (const level of ['高', '中']) {
    for (const a of advised.filter((x) => x.level === level)) console.log(`| ${level} | \`${a.file}\` | ${a.title} | （从条目里抄一句） |`)
  }
  process.exit(0)
}
for (const [file, entries] of doc) {
  for (const e of entries) {
    if (!VERDICTS.includes(e.verdict)) problems.push(`${file} · 「${e.title}」：「要不要改」要写成 ${VERDICTS.join(' / ')} 之一`)
  }
}
if (!flag('--partial')) {
  const key = (x) => `${x.level}|${x.file}|${x.title}`
  const listed = new Set(summary.map(key))
  for (const a of advised) if (!listed.has(key(a))) problems.push(`「要改的地方」少了：${a.level} · ${a.file} · ${a.title}`)
  const wanted = new Set(advised.map(key))
  for (const r of summary) if (!wanted.has(key(r))) problems.push(`「要改的地方」多了（条目不在了或级别变了）：${r.level} · ${r.file} · ${r.title}`)
}
const uncovered = new Map()
for (const [file, list] of sites) {
  if (flag('--partial') && !doc.has(file)) continue
  const source = readFileSync(join(ROOT, file), 'utf8')
    .split('\n')
    .map((t, i) => ({ line: i + 1, text: t.trim() }))
    .filter((s) => s.text)
  const covered = new Set()
  for (const entry of doc.get(file) ?? []) {
    let shows = 0
    for (const snippet of entry.snippets) {
      const where = locate(snippet, source)
      if (where.error) {
        problems.push(`${file} · 「${entry.title}」（文档第 ${snippet.line} 行）：${where.error}`)
        continue
      }
      for (const s of list) if (where.lines.has(s.line)) (covered.add(s), shows++)
    }
    if (!entry.snippets.length) problems.push(`${file} · 「${entry.title}」：没有引用代码`)
    else if (!shows && !entry.manual) problems.push(`${file} · 「${entry.title}」：引用的代码里已经没有要说明的写法了，删掉或更新这一条（人工检查的条目标题里写「（人工）」）`)
  }
  const missing = list.filter((s) => !covered.has(s))
  if (missing.length) uncovered.set(file, { missing, source })
}
for (const file of doc.keys()) {
  if (!existsSync(join(ROOT, file))) problems.push(`${file}：文件已经不在了，删掉这一节`)
  else if (!sites.has(file) && !(doc.get(file) ?? []).every((e) => e.manual)) problems.push(`${file}：这个文件已经没有要说明的写法了，删掉这一节`)
}

if (flag('--skeleton')) {
  for (const [file, { missing, source }] of uncovered) {
    console.log(`## \`${file}\`\n`)
    for (const s of missing) {
      console.log(`### ${s.kind}：（写一句这是在做什么）\n\n\`\`\`${lang(file)}\n${suggest(s, source).join('\n')}\n\`\`\`\n`)
      console.log('- **为什么这样写**：\n- **什么时候会出事**：\n- **要不要改**：\n')
    }
  }
  process.exit(0)
}

for (const [, { missing }] of uncovered) {
  for (const s of missing) problems.push(`${s.file}:${s.line} [${s.kind}] 还没写进文档：${s.text}`)
}
const total = [...sites.values()].reduce((n, l) => n + l.length, 0)
if (problems.length) {
  console.error(`STABILITY.md 和代码不一致（${problems.length} 处）：\n`)
  for (const p of problems) console.error(`  ${p}`)
  console.error('\n新写法的条目可以用 node scripts/stability.mjs --skeleton 生成，再补上说明。')
  process.exit(1)
}
console.log(`STABILITY.md 和代码一致：${total} 处，${sites.size} 个文件。`)
