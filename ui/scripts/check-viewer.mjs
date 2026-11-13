// Opens the map viewer in headless Chrome and walks through it: levels,
// search, selection, the trace view, panning; then the last commit drawn
// on the map: the list of changes, stepping, a review mark, cutting the
// map down to the change. Prints what it measured and
// exits with 1 when a step fails or the page logs an error.
//
//   node scripts/check-viewer.mjs http://127.0.0.1:7878 [path/to/chrome]
//
// `.github/scripts/check-viewer.sh` at the repository root starts a server and runs this.

import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const origin = process.argv[2]
const browser =
  process.argv[3] ??
  [
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
  ].find((path) => existsSync(path))
if (!origin || !browser) {
  console.error(origin ? 'no Chrome or Chromium found: pass its path as the second argument' : 'usage: check-viewer.mjs <origin> [chrome]')
  process.exit(2)
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const port = 9300 + Math.floor(Math.random() * 500)
const profile = mkdtempSync(join(tmpdir(), 'mapdiff-viewer-check-'))
const chrome = spawn(
  browser,
  ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--no-first-run', '--window-size=1440,900', 'about:blank'],
  { stdio: ['ignore', 'ignore', 'pipe'] },
)
// Kept to explain a Chrome that does not come up.
let chromeSaid = ''
chrome.stderr.on('data', (chunk) => {
  chromeSaid = (chromeSaid + chunk).slice(-2000)
})

// What runs in the page. Each step returns a line to print, or throws.
const walk = async () => {
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const key = (name) => window.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true }))
  const stats = () => document.querySelector('[data-testid=map-stats]')
  const settled = async () => {
    await wait(60)
    for (let tries = 0; stats().textContent.startsWith('Laying') && tries < 800; tries++) await wait(50)
    await wait(350)
  }
  const drawn = () => document.querySelectorAll('.mbox').length
  const must = (truth, message) => {
    if (!truth) throw new Error(message)
  }
  const lines = []
  try {
  await settled()
  must(drawn() > 10, 'the map drew no boxes')
  const references = () => document.querySelector('[data-testid=map-refs]')
  for (let tries = 0; !references() && tries < 100; tries++) await wait(50)
  must(references()?.textContent.startsWith('References: by '), 'the bar does not say how references are found')
  lines.push(references().textContent.replace('Index', '').trim())
  lines.push(`package: ${stats().textContent}. ${stats().title}`)

  for (const [name, digit] of [['module', '3'], ['code', '4'], ['architecture', '1']]) {
    key(digit)
    await settled()
    must(document.querySelector('.map-levels .is-current').textContent.toLowerCase() === name, `level ${name} was not reached`)
    must(drawn() > 0, `nothing drawn at ${name}`)
    lines.push(`${name}: ${stats().textContent}. ${stats().title}`)
  }
  must(document.querySelectorAll('.mbox.d-pill').length > 0, 'no files drawn as small marks at the architecture level')
  must(document.querySelectorAll('.mwire path').length > 0, 'no wires drawn at the architecture level')

  // Search for a function and jump to it: the level changes to show it.
  const hits = (await (await fetch('/api/search?q=main')).json()).hits
  const wanted = hits.find((hit) => hit.kind === 'function') ?? hits[0]
  must(wanted, 'search found nothing for "main"')
  key('/')
  await wait(150)
  const input = document.querySelector('.msearch input')
  must(input, 'the search box did not open')
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, wanted.label)
  input.dispatchEvent(new Event('input', { bubbles: true }))
  for (let tries = 0; !document.querySelector('.msearch li button') && tries < 40; tries++) await wait(50)
  const first = document.querySelector('.msearch li button')
  must(first, 'the search listed nothing')
  first.click()
  await settled()
  for (let tries = 0; !document.querySelector('.minspect') && tries < 60; tries++) await wait(50)
  const selected = document.querySelector('.mbox.is-selected')
  must(selected, 'the search result was not selected on the map')
  must(document.querySelector('.minspect'), 'the panel for the selection did not open')
  lines.push(`search: jumped to ${selected.getAttribute('data-id')}; path shown: ${document.querySelector('.map-crumbs').textContent}`)
  const lit = document.querySelectorAll('.mbox.is-near').length
  const links = document.querySelectorAll('.mlink').length
  lines.push(`selection: ${lit} boxes lit, ${links} paths drawn to and from it`)

  // The trace view shows its code.
  key('t')
  for (let tries = 0; document.querySelectorAll('.mtrace-line').length === 0 && tries < 80; tries++) await wait(50)
  const traced = document.querySelectorAll('.mtrace-line').length
  must(traced > 0, 'the trace view showed no code')
  await wait(200)
  lines.push(`trace: ${traced} lines of code, ${document.querySelectorAll('.mtrace-card').length} cards, ${document.querySelectorAll('.mtrace-curve').length} lines to them`)
  key('Escape')
  await wait(100)
  must(!document.querySelector('.mtrace'), 'the trace view did not close')

  // Code text appears in boxes at the code level once it is large enough.
  key('4')
  await settled()
  // The box is as wide on screen as it was; bring it to reading size.
  const show = [...document.querySelectorAll('.minspect button')].find((button) => button.textContent === 'Show on map')
  must(show, 'the panel has no button to show the selection')
  show.click()
  await wait(700)
  for (let tries = 0; document.querySelectorAll('.mcode-text span').length === 0 && tries < 60; tries++) await wait(50)
  lines.push(`code: ${document.querySelectorAll('.mcode-text').length} boxes show their text`)
  must(document.querySelectorAll('.mcode-text').length > 0, 'no code text at the code level')

  // Pan around the middle of the largest level for three seconds.
  key('Escape')
  key('3')
  await settled()
  key('0')
  await wait(500)
  const frame = document.querySelector('.mframe')
  const wheel = (init) => frame.dispatchEvent(new WheelEvent('wheel', { clientX: 600, clientY: 400, bubbles: true, cancelable: true, ...init }))
  for (let step = 0; step < 20; step++) wheel({ deltaY: -8, ctrlKey: true })
  await wait(300)
  // Zooming in may have opened the next level; measure where that leaves us.
  await settled()
  let frames = 0
  let worst = 0
  let last = 0
  const started = performance.now()
  await new Promise((done) => {
    const step = (now) => {
      if (last) worst = Math.max(worst, now - last)
      last = now
      frames++
      const way = Math.floor(frames / 45) % 2 ? -1 : 1
      wheel({ deltaX: way * 36, deltaY: way * 22 })
      if (now - started < 3000) requestAnimationFrame(step)
      else done()
    }
    requestAnimationFrame(step)
  })
  lines.push(`panning: ${Math.round(frames / 3)} frames a second, slowest frame ${Math.round(worst)} ms, with ${stats().textContent}`)
  must(worst < 250, `a frame took ${Math.round(worst)} ms while panning`)
  } catch (problem) {
    return { lines, failure: problem.message }
  }
  return { lines, failure: null }
}

// The same page with a comparison drawn on it: the last commit.
const review = async () => {
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const key = (name) => window.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true }))
  const stats = () => document.querySelector('[data-testid=map-stats]')
  const settled = async () => {
    await wait(60)
    for (let tries = 0; stats().textContent.startsWith('Laying') && tries < 800; tries++) await wait(50)
    await wait(350)
  }
  const must = (truth, message) => {
    if (!truth) throw new Error(message)
  }
  const toned = () => document.querySelectorAll('.mbox.t-added, .mbox.t-removed, .mbox.t-modified, .mbox.t-moved').length
  const lines = []
  try {
    await settled()
    const rows = document.querySelectorAll('.mchanges-row').length
    must(rows > 0, 'the list of changes is empty')
    must(toned() + document.querySelectorAll('.mbox.holds-change').length > 0, 'no box on the map is marked as changed')
    lines.push(`changes: ${rows} listed; ${toned()} boxes coloured, ${document.querySelectorAll('.mchip').length} chips, at the package level`)

    key('3')
    await settled()
    key('n')
    // The view moves to the change; its box is drawn once it gets there.
    for (let tries = 0; !(document.querySelector('.mdock table.diff') && document.querySelector('.mbox.is-selected')) && tries < 100; tries++) await wait(50)
    const selected = document.querySelector('.mbox.is-selected')
    must(selected, 'stepping to the first change selected nothing')
    must(document.querySelector('.mchanges-row.is-current'), 'the list does not show which change is being read')
    must(document.querySelector('.mdock table.diff'), 'no before and after under the map for the change being read')
    must(document.querySelector('.minspect-change'), 'the panel does not describe the change')
    lines.push(`first change: ${selected.getAttribute('data-id')}; before and after shown in ${document.querySelectorAll('.mdock table.diff tr').length} rows`)

    // A review mark, set and taken off again.
    const before = document.querySelector('.mchanges-progress').textContent
    key('r')
    for (let tries = 0; document.querySelector('.mchanges-progress').textContent === before && tries < 60; tries++) await wait(50)
    const marked = document.querySelector('.mchanges-progress').textContent
    must(marked !== before, 'marking a change as reviewed changed nothing')
    key('r')
    for (let tries = 0; document.querySelector('.mchanges-progress').textContent !== before && tries < 60; tries++) await wait(50)
    must(document.querySelector('.mchanges-progress').textContent === before, 'the review mark could not be taken off')
    lines.push(`review mark: ${before} -> ${marked} -> ${before}`)

    // Cut the map down to the change.
    const whole = Number(stats().textContent.match(/of (\d+)/)[1])
    const select = document.querySelector('select[aria-label="How much of the map"]')
    const choose = async (value) => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, value)
      select.dispatchEvent(new Event('change', { bubbles: true }))
      await settled()
      return Number(stats().textContent.match(/of (\d+)/)[1])
    }
    const changed = await choose('changed')
    const near = await choose('near')
    must(changed < whole && changed > 0, `cutting the map to the change left ${changed} of ${whole} boxes`)
    must(near >= changed && near <= whole, `with neighbours there are ${near} boxes, between ${changed} and ${whole} expected`)
    lines.push(`cut down: ${whole} boxes, ${changed} with only what changed, ${near} with what touches it`)

    // Code level: the text of the changed function, with what was added marked.
    await choose('all')
    key('4')
    await settled()
    key('n')
    await wait(400)
    key('p')
    for (let tries = 0; document.querySelectorAll('.mbox.is-selected .mcode-text span').length === 0 && tries < 100; tries++) await wait(50)
    // The tint follows the before and after of the change, which arrives a moment later.
    for (let tries = 0; document.querySelectorAll('.mcode-text .is-inserted').length === 0 && tries < 40; tries++) await wait(50)
    lines.push(`code: ${document.querySelectorAll('.mcode-text .is-inserted').length} added lines tinted in the box`)

    // A long block is cut; its last line opens the rest in place.
    const cutBox = document.querySelector('.mbox [data-more]')
    if (cutBox) {
      const id = cutBox.getAttribute('data-more')
      const heightOf = () => document.querySelector(`.mbox[data-id="${CSS.escape(id)}"]`)?.getBoundingClientRect().height ?? 0
      const scale = () => new DOMMatrix(getComputedStyle(document.querySelector('.mworld')).transform).a
      const cut = heightOf() / scale()
      cutBox.click()
      await settled()
      for (let tries = 0; heightOf() / scale() <= cut + 1 && tries < 60; tries++) await wait(50)
      const whole = heightOf() / scale()
      must(whole > cut + 1, 'opening a cut block did not make its box taller')
      lines.push(`open in place: a block grew from ${Math.round(cut)} to ${Math.round(whole)} map pixels high`)
    } else lines.push('open in place: no cut block on screen to try')
  } catch (problem) {
    return { lines, failure: problem.message }
  }
  return { lines, failure: null }
}

let failed = false
try {
  let target
  // A busy CI machine can take most of a minute to start Chrome.
  for (let tries = 0; tries < 300 && !target; tries++) {
    await sleep(200)
    try {
      target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((each) => each.type === 'page')
    } catch {
      // Chrome is still starting.
    }
  }
  if (!target) throw new Error(`Chrome did not start\n${chromeSaid}`)
  const socket = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((resolve) => (socket.onopen = resolve))
  let next = 1
  const waiting = new Map()
  const errors = []
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data)
    if (message.id && waiting.has(message.id)) {
      waiting.get(message.id)(message)
      waiting.delete(message.id)
    } else if (message.method === 'Runtime.exceptionThrown') {
      errors.push(message.params.exceptionDetails.exception?.description ?? message.params.exceptionDetails.text)
    } else if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
      errors.push(message.params.args.map((arg) => arg.value ?? arg.description).join(' '))
    }
  }
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const id = next++
      waiting.set(id, resolve)
      socket.send(JSON.stringify({ id, method, params }))
    })
  await send('Runtime.enable')
  await send('Page.enable')
  await send('Page.navigate', { url: `${origin}/#map` })
  await sleep(1500)
  const answer = await send('Runtime.evaluate', {
    expression: `(${walk.toString()})()`,
    awaitPromise: true,
    returnByValue: true,
  })
  const report = (answer) => {
    if (answer.result.exceptionDetails) {
      failed = true
      console.error(`FAILED: ${answer.result.exceptionDetails.exception?.description ?? answer.result.exceptionDetails.text}`)
      return
    }
    const { lines, failure } = answer.result.result.value
    for (const line of lines) console.log(line)
    if (failure) {
      failed = true
      console.error(`FAILED: ${failure}`)
    }
  }
  report(answer)

  // Again, with the last commit drawn on the map.
  await send('Page.navigate', { url: `${origin}/?mode=commit&commit=HEAD#map/changes` })
  await send('Page.reload')
  await sleep(1500)
  report(
    await send('Runtime.evaluate', {
      expression: `(${review.toString()})()`,
      awaitPromise: true,
      returnByValue: true,
    }),
  )
  if (errors.length > 0) {
    failed = true
    console.error(`the page logged ${errors.length} errors:`)
    for (const error of errors.slice(0, 10)) console.error(`  ${error}`)
  }
  socket.close()
} catch (problem) {
  failed = true
  console.error(`FAILED: ${problem.message}`)
} finally {
  chrome.kill()
  await sleep(400)
  rmSync(profile, { recursive: true, force: true })
}
process.exit(failed ? 1 : 0)
