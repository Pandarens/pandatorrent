/**
 * What the catalogue hands to the search view when a section is picked.
 *
 * `forumIds` is separate from `id` on purpose: browsing a parent forum such as
 * "Игры для Windows" has to include its subforums, because the parent itself
 * holds no releases and would otherwise come back empty.
 */
export interface BrowseTarget {
  id: number
  title: string
  forumIds: number[]
}
