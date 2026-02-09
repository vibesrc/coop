import { useState, useCallback } from 'react'

const LAYOUT_KEY = 'coop_layout'

export interface LayoutState {
  sidebarOpen: boolean
  splitSizes: number[]
}

const defaultLayout: LayoutState = {
  sidebarOpen: true,
  splitSizes: [100],
}

function loadLayout(): LayoutState {
  try {
    const stored = localStorage.getItem(LAYOUT_KEY)
    if (stored) return { ...defaultLayout, ...JSON.parse(stored) }
  } catch {
    // ignore
  }
  return defaultLayout
}

function saveLayout(state: LayoutState) {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify(state))
  } catch {
    // ignore
  }
}

export function useLayout() {
  const [layout, setLayoutState] = useState(loadLayout)

  const setLayout = useCallback((update: Partial<LayoutState>) => {
    setLayoutState((prev) => {
      const next = { ...prev, ...update }
      saveLayout(next)
      return next
    })
  }, [])

  return { layout, setLayout }
}
