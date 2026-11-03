// A task from GitHub Issues / Azure DevOps, as orcha's tracker.py returns it.
export type WorkItem = {
  id: string
  title: string
  state: string
  type: string
  assignee: string | null
  summary: string
  url: string
}

export type BranchRow = {
  name: string
  id: string | null
  title: string | null
  state: string | null
  isPushed: boolean
  unpushed: number
  ahead: number
  behind: number
  merged: 'yes' | 'pr' | 'no'
  age: string
  // the branch ahead / behind are counted against, origin/<base>
  base: string
}

// Your own commits today, across all branches, and the run of days with one.
export type DayStats = {
  commits: number
  added: number
  removed: number
  streak: number
  // hour of today's first and last commit (local time), null with no commit
  firstHour: number | null
  lastHour: number | null
  // the repo's check passed today without Claude having to fix anything
  isCleanCheck: boolean
}

// What the task card above the prompt shows for the current branch.
export type TaskInfo = {
  today: DayStats
  repo: string
  branch: string
  id: string | null
  item: WorkItem | null
  // null: not on origin at all
  unpushed: number | null
  isProtected: boolean
}

// ~/.config/orcha/gen/plugin.json, written by `orcha apply`.
export type OrchaConfig = {
  home: string
  cache: string
  colors: Record<'accent' | 'red' | 'green' | 'blue' | 'yellow' | 'magenta' | 'muted' | 'fg' | 'peach', string>
  // a string, or a list to pick one from at random
  strings: Record<string, string | string[]>
  repo: { branch_prefix: string; base: string; protected: string }
  widgets: { card: boolean; cheers: boolean }
}

declare module 'claude-code' {
  interface PluginState {
    orcha: {
      branches: BranchRow[]
      isLoading: boolean
      updatedAt: number | null
      task: TaskInfo | null
      isCardHidden: boolean
    }
  }
}
