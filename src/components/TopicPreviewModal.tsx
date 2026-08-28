// A tracker topic, rendered the way the site presents it: formatted
// description with its line breaks intact, screenshots as a gallery, extra
// material folded into spoilers, and replies on their own tab.

import { useState } from 'react'
import { useEffect } from 'react'

import { tracker } from '../lib/api'
import { formatBytes } from '../lib/format'
import { useStore } from '../lib/store'
import type { PostBlock, TopicDetails } from '../lib/types'
import { Empty, Modal, Spinner } from './ui'

export function TopicPreviewModal({
  topicId,
  onClose,
  onDownload,
}: {
  topicId: number
  onClose: () => void
  onDownload: () => void
}) {
  const { reportError } = useStore()
  const [topic, setTopic] = useState<TopicDetails | null>(null)
  const [tab, setTab] = useState<'about' | 'comments'>('about')
  const [zoomed, setZoomed] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    tracker
      .topic(topicId)
      .then((t) => {
        if (!cancelled) setTopic(t)
      })
      .catch((e) => {
        if (!cancelled) reportError(e, 'Описание раздачи')
      })
    return () => {
      cancelled = true
    }
  }, [topicId, reportError])

  const comments = topic?.comments ?? []

  return (
    <>
      <Modal
        title={topic?.title ?? 'Загрузка…'}
        icon="📄"
        wide
        onClose={onClose}
        footer={
          <>
            <button className="btn ghost" onClick={onClose}>
              Закрыть
            </button>
            <button className="btn primary" onClick={onDownload}>
              ⬇ Скачать
            </button>
          </>
        }
      >
        {!topic ? (
          <Spinner />
        ) : (
          <>
            <div className="tabs">
              <button
                className={tab === 'about' ? 'tab active' : 'tab'}
                onClick={() => setTab('about')}
              >
                Описание
              </button>
              <button
                className={tab === 'comments' ? 'tab active' : 'tab'}
                onClick={() => setTab('comments')}
              >
                Комментарии
                {comments.length > 0 && <span className="tab-count">{comments.length}</span>}
              </button>
            </div>

            {tab === 'about' ? (
              <>
                {topic.images.length > 0 && (
                  <div className="topic-gallery">
                    {topic.images.slice(0, 12).map((url) => (
                      <button key={url} onClick={() => setZoomed(url)} title="Увеличить">
                        <img src={url} alt="" loading="lazy" />
                      </button>
                    ))}
                  </div>
                )}

                <dl className="kv" style={{ marginBottom: 14 }}>
                  <dt>Размер</dt>
                  <dd>{formatBytes(topic.sizeBytes)}</dd>
                  <dt>Info hash</dt>
                  <dd className="mono">{topic.infoHash ?? '—'}</dd>
                </dl>

                <Blocks blocks={topic.blocks} onZoom={setZoomed} />
              </>
            ) : comments.length === 0 ? (
              <Empty icon="💬" title="Комментариев нет" />
            ) : (
              <div className="comments">
                {comments.map((c, i) => (
                  <div className="comment" key={i}>
                    <div className="comment-head">
                      <strong>{c.author ?? 'аноним'}</strong>
                      {c.postedAt && <span className="page-sub">{c.postedAt}</span>}
                    </div>
                    <Blocks blocks={c.blocks} onZoom={setZoomed} />
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </Modal>

      {zoomed && (
        <div className="lightbox" onClick={() => setZoomed(null)}>
          <img src={zoomed} alt="" />
        </div>
      )}
    </>
  )
}

/** Renders a post tree: text keeps its line breaks, spoilers stay folded. */
function Blocks({
  blocks,
  onZoom,
}: {
  blocks: PostBlock[]
  onZoom: (url: string) => void
}) {
  return (
    <>
      {blocks.map((block, i) => {
        if (block.kind === 'text') {
          return (
            <p className="topic-text" key={i}>
              {block.text}
            </p>
          )
        }
        if (block.kind === 'image') {
          return (
            <button className="topic-inline-img" key={i} onClick={() => onZoom(block.url)}>
              <img src={block.url} alt="" loading="lazy" />
            </button>
          )
        }
        return <Spoiler key={i} title={block.title} blocks={block.blocks} onZoom={onZoom} />
      })}
    </>
  )
}

function Spoiler({
  title,
  blocks,
  onZoom,
}: {
  title: string
  blocks: PostBlock[]
  onZoom: (url: string) => void
}) {
  // Folded by default, matching the site — that is the point of a spoiler.
  const [open, setOpen] = useState(false)
  return (
    <div className="spoiler">
      <button className="spoiler-head" onClick={() => setOpen((v) => !v)}>
        <span className="chevron">{open ? '▾' : '▸'}</span>
        <span>{title}</span>
      </button>
      {open && (
        <div className="spoiler-body">
          <Blocks blocks={blocks} onZoom={onZoom} />
        </div>
      )}
    </div>
  )
}
