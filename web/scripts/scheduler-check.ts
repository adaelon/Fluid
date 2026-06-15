// S8 validation (B2, deterministic). Exercises the pure scheduling core —
// viewportDistance + PendingQueue — with assertions, no sockets / browser /
// network. Run with: node scripts/scheduler-check.ts  (Node 24 strips TS types).
//
// The parallel-socket concurrency and scroll wiring in GenScheduler are
// browser-verified (A2); this script locks the orderable logic those rely on.

import { viewportDistance, PendingQueue, type FnId } from '../src/scheduler.ts'

let failures = 0
function check(label: string, cond: boolean): void {
  if (cond) {
    console.log(`  PASS  ${label}`)
  } else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}
function eqArr(a: readonly FnId[], b: FnId[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i])
}

console.log('=== viewportDistance ===')
const vp = { fromLine: 10, toLine: 20 }
check('in-view start line → 0', viewportDistance(15, vp) === 0)
check('viewport edges → 0', viewportDistance(10, vp) === 0 && viewportDistance(20, vp) === 0)
check('above viewport → distance to top', viewportDistance(4, vp) === 6)
check('below viewport → distance to bottom', viewportDistance(27, vp) === 7)
check('further below sorts larger', viewportDistance(30, vp) > viewportDistance(25, vp))

console.log('\n=== PendingQueue.set orders by ascending distance ===')
// Functions at lines 1, 50, 14, 22; viewport [10,20] → distances 9,30,0,2.
const dist = new Map<FnId, number>([
  ['a#1', viewportDistance(1, vp)], // 9
  ['b#50', viewportDistance(50, vp)], // 30
  ['c#14', viewportDistance(14, vp)], // 0 (in view)
  ['d#22', viewportDistance(22, vp)], // 2
])
const q = new PendingQueue()
q.set(['a#1', 'b#50', 'c#14', 'd#22'], dist)
check('nearest (in-view) first', eqArr(q.order, ['c#14', 'd#22', 'a#1', 'b#50']))
check('shift returns nearest', q.shift() === 'c#14')
check('size decremented after shift', q.size === 3)

console.log('\n=== reprioritize on scroll ===')
// Scroll down so viewport is now [40,55]; b#50 becomes in-view (0).
const vp2 = { fromLine: 40, toLine: 55 }
const dist2 = new Map<FnId, number>([
  ['d#22', viewportDistance(22, vp2)], // 18
  ['a#1', viewportDistance(1, vp2)], // 39
  ['b#50', viewportDistance(50, vp2)], // 0
])
q.reprioritize(dist2)
check('scrolled-to function jumps ahead', eqArr(q.order, ['b#50', 'd#22', 'a#1']))

console.log('\n=== unknown ids sort last ===')
const q2 = new PendingQueue()
q2.set(['x', 'y', 'z'], new Map([['y', 5]]))
check('id with known small distance leads', q2.order[0] === 'y')
check('unknown ids keep stable relative order after the known one', eqArr(q2.order, ['y', 'x', 'z']))

console.log('\n=== retry jumps to front ===')
const q3 = new PendingQueue()
q3.set(['p', 'r'], new Map([['p', 1], ['r', 2]]))
q3.shift() // dispatch p
q3.pushFront('p') // p failed → retry
check('retried id is at front', q3.order[0] === 'p')
check('no duplicate on double pushFront', (q3.pushFront('p'), q3.order.filter((i) => i === 'p').length === 1))

console.log(`\n${failures === 0 ? 'ALL PASS' : failures + ' FAILED'}`)
process.exit(failures === 0 ? 0 : 1)
