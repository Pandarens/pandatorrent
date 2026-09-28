// How the download list is ordered.
//
// Pulled out of the view so the comparator can be tested on its own: sorting
// by progress or speed reads through the live stats, and a row that has none
// yet must still land somewhere sensible.

export type SortKey = 'added' | 'name' | 'size' | 'progress' | 'speed'

export const SORT_LABELS: Record<SortKey, string> = {
  added: 'По добавлению',
  name: 'По названию',
  size: 'По размеру',
  progress: 'По прогрессу',
  speed: 'По скорости',
}

/** The slice of a download row the comparator needs. */
export interface SortableRow {
  t: { name: string; addedAt: number; totalBytes: number }
  p: { totalBytes: number; progressBytes: number; downloadSpeedBps: number } | null
}

export function compareRows(sort: SortKey, descending: boolean) {
  const size = (r: SortableRow) => r.p?.totalBytes || r.t.totalBytes
  const share = (r: SortableRow) => {
    const total = size(r)
    return total > 0 ? (r.p?.progressBytes ?? 0) / total : 0
  }
  return (a: SortableRow, b: SortableRow): number => {
    let by = 0
    switch (sort) {
      case 'name':
        by = a.t.name.localeCompare(b.t.name, 'ru')
        break
      case 'size':
        by = size(a) - size(b)
        break
      case 'progress':
        by = share(a) - share(b)
        break
      case 'speed':
        by = (a.p?.downloadSpeedBps ?? 0) - (b.p?.downloadSpeedBps ?? 0)
        break
      default:
        by = a.t.addedAt - b.t.addedAt
    }
    return descending ? -by : by
  }
}
