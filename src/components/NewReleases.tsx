// "What is new" strips on the home screen.
//
// Each strip is one tracker forum browsed newest-first — the same search path
// the rest of the app uses, so nothing here needs its own parser. Which forums
// appear, and how many items each shows, is configured in settings; the backend
// caches the result for half an hour so opening the library is not a burst of
// tracker requests.

import { useEffect, useState } from 'react'

import { tracker } from '../lib/api'
import { useStore } from '../lib/store'
import type { BrowseTarget } from '../lib/browse'
import type { NewReleaseSection } from '../lib/types'
import { ResultCard, useTrackerDownload } from './ResultCard'
import { ScrollStrip } from './ScrollStrip'
import { TopicPreviewModal } from './TopicPreviewModal'
import { TrackerLogin } from './TrackerLogin'
import { Spinner } from './ui'

export function NewReleases({
  onOpenGame,
  onOpenSection,
}: {
  onOpenGame: (infoHash: string) => void
  /** Browse the whole forum a strip is a window onto. */
  onOpenSection?: (target: BrowseTarget) => void
}) {
  const { config, reportError } = useStore()

  const [sections, setSections] = useState<NewReleaseSection[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [needLogin, setNeedLogin] = useState(false)
  const [preview, setPreview] = useState<number | null>(null)

  const { downloading, download } = useTrackerDownload({
    onAdded: onOpenGame,
    onNeedLogin: () => setNeedLogin(true),
  })

  const enabled = config?.home.enabled ?? false
  // Re-fetch when the pinned list changes, so settings take effect at once.
  const signature = (config?.home.forums ?? []).map((f) => f.id).join(',')

  async function load(refresh = false) {
    setLoading(true)
    try {
      setSections(await tracker.newReleases(refresh))
    } catch (e) {
      reportError(e, 'Новинки')
      setSections([])
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (!enabled || !signature) {
      setSections([])
      return
    }
    void load()
  }, [enabled, signature])

  if (!enabled || !signature) return null
  if (sections === null) {
    return (
      <div className="strip-block">
        <div className="strip-head">
          <h2 className="strip-title">Новинки</h2>
          <Spinner />
        </div>
      </div>
    )
  }

  const anything = sections.some((s) => s.items.length > 0 || s.error)
  if (!anything) return null

  return (
    <>
      {sections.map((section) => (
        <div className="strip-block" key={section.forumId}>
          <div className="strip-head">
            <h2 className="strip-title">{section.forumTitle}</h2>
            <div className="spacer" />
            {onOpenSection && (
              <button
                className="btn ghost sm"
                title={`Открыть раздел «${section.forumTitle}» целиком`}
                onClick={() =>
                  onOpenSection({
                    id: section.forumId,
                    title: section.forumTitle,
                    forumIds: [section.forumId],
                  })
                }
              >
                Показать ещё
              </button>
            )}
            <button
              className="btn ghost sm"
              onClick={() => void load(true)}
              disabled={loading}
              title="Обновить новинки"
            >
              {loading ? <Spinner /> : '⟳'}
            </button>
          </div>

          {section.error ? (
            <div className="banner warn" style={{ marginBottom: 0 }}>
              <span>⚠</span>
              <span>Не удалось загрузить раздел: {section.error}</span>
            </div>
          ) : (
            <ScrollStrip>
              {section.items.map((item) => (
                <ResultCard
                  key={item.topicId}
                  item={item}
                  downloading={downloading === item.topicId}
                  onPreview={() => setPreview(item.topicId)}
                  onDownload={() => download(item)}
                />
              ))}
            </ScrollStrip>
          )}
        </div>
      ))}

      {needLogin && <TrackerLogin onClose={() => setNeedLogin(false)} />}
      {preview != null && (
        <TopicPreviewModal
          topicId={preview}
          onClose={() => setPreview(null)}
          onDownload={() => {
            const item = sections
              .flatMap((s) => s.items)
              .find((i) => i.topicId === preview)
            setPreview(null)
            if (item) void download(item)
          }}
        />
      )}
    </>
  )
}
