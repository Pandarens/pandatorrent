// The tracker's section tree, for browsing without typing a query.
//
// The whole tree — categories, forums and subforums such as "Новинки" — comes
// from a single cached `index.php` parse, so expanding a category costs nothing
// and nothing is missing from the list.

import { useEffect, useMemo, useState } from 'react'

import { tracker } from '../lib/api'
import { useStore } from '../lib/store'
import type { BrowseTarget } from '../lib/browse'
import type { CatalogCategory } from '../lib/types'
import { Spinner } from './ui'

export function CatalogPanel({
  activeForumId,
  onPick,
}: {
  activeForumId: number | null
  onPick: (target: BrowseTarget) => void
}) {
  const { reportError } = useStore()

  const [categories, setCategories] = useState<CatalogCategory[] | null>(null)
  const [open, setOpen] = useState<Set<number>>(new Set())
  const [filter, setFilter] = useState('')
  const [error, setError] = useState<string | null>(null)

  async function load(refresh = false) {
    setError(null)
    setCategories(null)
    try {
      const tree = await tracker.catalog(refresh)
      setCategories(tree)
      setOpen(new Set(tree.slice(0, 1).map((c) => c.id)))
    } catch (e) {
      setError(reportError(e, 'Каталог').message)
    }
  }

  useEffect(() => {
    void load()
  }, [])

  // Filtering matches subforums too, and keeps the parent visible so a hit like
  // "Новинки" is still readable in context.
  const shown = useMemo(() => {
    if (!categories) return null
    const q = filter.trim().toLowerCase()
    if (!q) return categories

    return categories
      .map((category) => ({
        ...category,
        forums: category.forums.filter(
          (f) =>
            f.title.toLowerCase().includes(q) ||
            f.subforums.some((s) => s.title.toLowerCase().includes(q)),
        ),
      }))
      .filter((c) => c.forums.length > 0 || c.title.toLowerCase().includes(q))
  }, [categories, filter])

  const filtering = filter.trim().length > 0

  return (
    <aside className="catalog">
      <div className="catalog-head">
        <span>Разделы трекера</span>
        <button className="btn ghost sm" title="Обновить каталог" onClick={() => void load(true)}>
          ⟳
        </button>
      </div>

      {categories && categories.length > 6 && (
        <input
          className="input"
          style={{ marginBottom: 8 }}
          placeholder="Фильтр разделов…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      )}

      {error && <div className="catalog-error">{error}</div>}
      {!categories && !error && (
        <div className="catalog-loading">
          <Spinner /> Загрузка каталога…
        </div>
      )}

      {shown?.length === 0 && <div className="catalog-loading">Ничего не найдено</div>}

      {shown?.map((category) => {
        const expanded = filtering || open.has(category.id)
        return (
          <div key={category.id} className="catalog-group">
            <button
              className="catalog-group-head"
              onClick={() =>
                setOpen((prev) => {
                  const next = new Set(prev)
                  if (next.has(category.id)) next.delete(category.id)
                  else next.add(category.id)
                  return next
                })
              }
            >
              <span className="chevron">{expanded ? '▾' : '▸'}</span>
              <span>{category.title}</span>
            </button>

            {expanded && (
              <div className="catalog-items">
                {category.forums.map((forum) => (
                  <div key={forum.id}>
                    <button
                      className={
                        activeForumId === forum.id ? 'catalog-item active' : 'catalog-item'
                      }
                      onClick={() =>
                        onPick({
                          id: forum.id,
                          title: forum.title,
                          // A parent forum usually holds nothing itself — the
                          // releases live in its subforums, so browse both.
                          forumIds: [forum.id, ...forum.subforums.map((s) => s.id)],
                        })
                      }
                      title={forum.title}
                    >
                      {forum.title}
                    </button>

                    {forum.subforums.map((sub) => (
                      <button
                        key={sub.id}
                        className={
                          activeForumId === sub.id
                            ? 'catalog-item nested active'
                            : 'catalog-item nested'
                        }
                        onClick={() =>
                          onPick({
                            id: sub.id,
                            title: `${forum.title} · ${sub.title}`,
                            forumIds: [sub.id],
                          })
                        }
                        title={`${forum.title} · ${sub.title}`}
                      >
                        • {sub.title}
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            )}
          </div>
        )
      })}
    </aside>
  )
}
