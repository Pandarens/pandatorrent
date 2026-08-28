// Fetches libmpv for the bundled player.
//
// The app loads libmpv at runtime rather than linking it, so this only has to
// place one DLL where the binary can find it. It is a build step, not part of
// the app: run `npm run fetch:mpv` once, or whenever you want a newer mpv.
//
// Source: https://github.com/shinchiro/mpv-winbuild-cmake — the project that
// produces the official mpv Windows builds. The plain x86_64 archive is used
// rather than the `-v3` one, which requires a CPU with AVX2.

import { execFileSync } from 'node:child_process'
import { createWriteStream, existsSync, mkdirSync, readdirSync, rmSync, statSync } from 'node:fs'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const target = path.join(root, 'src-tauri', 'resources', 'mpv')
const temp = path.join(root, 'node_modules', '.cache', 'mpv')

const RELEASES =
  'https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest'

/** 7-Zip, wherever it happens to live. */
function findSevenZip() {
  const candidates = [
    process.env.SEVENZIP,
    'C:/Program Files/7-Zip/7z.exe',
    'C:/Program Files (x86)/7-Zip/7z.exe',
    '7z',
  ].filter(Boolean)

  for (const candidate of candidates) {
    try {
      execFileSync(candidate, ['i'], { stdio: 'ignore' })
      return candidate
    } catch (e) {
      // `7z i` exits non-zero on some builds but still proves it runs.
      if (e.status !== undefined && candidate !== '7z') return candidate
    }
  }
  return null
}

async function download(url, dest) {
  const res = await fetch(url, {
    headers: { 'User-Agent': 'panda-torrent-build' },
    redirect: 'follow',
  })
  if (!res.ok) throw new Error(`${url} → HTTP ${res.status}`)
  await pipeline(Readable.fromWeb(res.body), createWriteStream(dest))
}

async function main() {
  const sevenZip = findSevenZip()
  if (!sevenZip) {
    console.error(
      'Не найден 7-Zip, а сборки mpv распространяются в .7z.\n' +
        'Установите 7-Zip (https://www.7-zip.org) либо скачайте libmpv-2.dll вручную\n' +
        `и положите её в ${target}`,
    )
    process.exit(1)
  }

  console.log('Ищу свежую сборку mpv…')
  const res = await fetch(RELEASES, { headers: { 'User-Agent': 'panda-torrent-build' } })
  if (!res.ok) throw new Error(`GitHub API → HTTP ${res.status}`)
  const release = await res.json()

  // Skip the `-v3` variant: it needs AVX2 and would crash on older CPUs.
  const asset = release.assets.find(
    (a) => a.name.startsWith('mpv-dev-x86_64-') && !a.name.includes('-v3-'),
  )
  if (!asset) throw new Error('в релизе нет архива mpv-dev-x86_64')

  mkdirSync(temp, { recursive: true })
  const archive = path.join(temp, asset.name)

  if (!existsSync(archive)) {
    console.log(`Скачиваю ${asset.name} (${(asset.size / 1048576).toFixed(1)} МБ)…`)
    await download(asset.browser_download_url, archive)
  } else {
    console.log(`Использую уже скачанный ${asset.name}`)
  }

  mkdirSync(target, { recursive: true })
  console.log('Распаковываю libmpv…')
  execFileSync(sevenZip, ['e', '-y', `-o${target}`, archive, 'libmpv-2.dll', 'mpv-2.dll'], {
    stdio: 'inherit',
  })

  const dlls = readdirSync(target).filter((f) => f.toLowerCase().endsWith('.dll'))
  if (dlls.length === 0) {
    rmSync(target, { recursive: true, force: true })
    throw new Error('в архиве не оказалось libmpv-2.dll')
  }
  for (const dll of dlls) {
    const size = statSync(path.join(target, dll)).size
    console.log(`Готово: ${dll} (${(size / 1048576).toFixed(1)} МБ) → ${target}`)
  }
}

main().catch((e) => {
  console.error(`Не удалось получить mpv: ${e.message}`)
  process.exit(1)
})
