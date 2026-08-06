// S-QDOCK-1 deterministic checks. Run with:
//   node scripts/query-layout-check.ts
// Node 24 strips TypeScript annotations; no Vue, DOM or backend is needed.

import {
  QUERY_DOCK_DEFAULT_PX,
  QUERY_DOCK_EDITOR_RESERVE_PX,
  QUERY_DOCK_MIN_PX,
  QUERY_DOCK_STATUS_BAR_PX,
  clampQueryDockHeight,
  loadQueryDockHeight,
  queryDockHeightBounds,
  queryDockHeightFromPointer,
} from '../src/queryLayout.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== query dock defaults and dynamic bounds ===')
check('default remains 240px', loadQueryDockHeight(null, 768) === QUERY_DOCK_DEFAULT_PX)
check('nominal minimum remains 160px', clampQueryDockHeight(40, 768) === QUERY_DOCK_MIN_PX)
check(
  'maximum reserves editor and status bar',
  queryDockHeightBounds(768).max ===
    768 - QUERY_DOCK_EDITOR_RESERVE_PX - QUERY_DOCK_STATUS_BAR_PX,
)
check('oversized requests clamp to the dynamic maximum', clampQueryDockHeight(900, 500) === 296)
check('fractional requests normalize to whole pixels', clampQueryDockHeight(240.6, 768) === 241)

console.log('\n=== persisted values are normalized on read ===')
check('empty storage uses the default', loadQueryDockHeight('', 768) === 240)
check('non-numeric storage uses the default', loadQueryDockHeight('not-a-size', 768) === 240)
check('partial CSS numbers are rejected', loadQueryDockHeight('480px', 768) === 240)
check('negative storage uses the default', loadQueryDockHeight('-40', 768) === 240)
check('zero storage uses the default', loadQueryDockHeight('0', 768) === 240)
check('non-finite storage uses the default', loadQueryDockHeight('Infinity', 768) === 240)
check('small positive storage clamps to the minimum', loadQueryDockHeight('80', 768) === 160)
check('large positive storage clamps to the maximum', loadQueryDockHeight('900', 500) === 296)

console.log('\n=== pointer movement maps to dock height ===')
check('dragging upward grows the dock', queryDockHeightFromPointer(240, 500, 380, 768) === 360)
check('dragging downward shrinks the dock', queryDockHeightFromPointer(240, 500, 600, 768) === 160)
check('dragging above the viewport clamps to max', queryDockHeightFromPointer(240, 500, -500, 500) === 296)
check('dragging below the viewport clamps to min', queryDockHeightFromPointer(240, 500, 1500, 768) === 160)

console.log('\n=== viewport shrink keeps the editor usable ===')
check('a saved 500px dock fits a tall viewport', loadQueryDockHeight('500', 768) === 500)
check('the same value reclamps after viewport shrink', loadQueryDockHeight('500', 480) === 276)
const shortBounds = queryDockHeightBounds(300)
check('short viewports relax the nominal dock minimum', shortBounds.min === 96)
check('short viewports still reserve editor plus status', shortBounds.max === 96)
check('no dock space remains below the editor reserve', clampQueryDockHeight(240, 180) === 0)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-layout checks passed.')
