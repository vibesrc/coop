import { useState, useCallback, useEffect, useRef } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import { SessionSidebar } from '@/components/session-sidebar'
import { PtyTabs, type PtyTab } from '@/components/pty-tabs'
import { Terminal } from '@/components/terminal'
import { useLayout } from '@/hooks/use-layout'
import type { Transport, Session, PtyConnection } from '@/transport'

interface AppLayoutProps {
  transport: Transport
}

interface PtyState {
  tab: PtyTab
  connection: PtyConnection | null
}

export function AppLayout({ transport }: AppLayoutProps) {
  const { layout, setLayout } = useLayout()

  // Sessions
  const [sessions, setSessions] = useState<Session[]>([])
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null)

  // PTY tabs per session
  const [ptyStates, setPtyStates] = useState<PtyState[]>([])
  const [activeTab, setActiveTab] = useState(0)

  // Track connections for cleanup
  const connectionsRef = useRef<PtyConnection[]>([])

  // Fetch sessions on mount and periodically
  const fetchSessions = useCallback(async () => {
    try {
      const list = await transport.listSessions()
      setSessions(list)
      // Auto-select first session if none selected
      if (list.length > 0) {
        setActiveSessionId((prev) =>
          prev && list.some((s) => s.id === prev) ? prev : list[0].id,
        )
      }
    } catch {
      // Transport not ready yet
    }
  }, [transport])

  useEffect(() => {
    fetchSessions()
    const interval = setInterval(fetchSessions, 5000)
    return () => clearInterval(interval)
  }, [fetchSessions])

  // When active session changes, set up agent PTY tab
  useEffect(() => {
    if (!activeSessionId) {
      setPtyStates([])
      return
    }

    // Clean up previous connections
    for (const conn of connectionsRef.current) {
      conn.close()
    }
    connectionsRef.current = []

    // Create agent tab (PTY 0)
    const agentTab: PtyTab = { id: 0, label: 'Agent', isAgent: true }
    setPtyStates([{ tab: agentTab, connection: null }])
    setActiveTab(0)

    // Attach to agent PTY
    const conn = transport.attachPty(activeSessionId, 0, 80, 24)
    connectionsRef.current.push(conn)
    setPtyStates([{ tab: agentTab, connection: conn }])

    return () => {
      for (const c of connectionsRef.current) {
        c.close()
      }
      connectionsRef.current = []
    }
  }, [activeSessionId, transport])

  const handleNewSession = useCallback(async () => {
    try {
      const name = `session-${Date.now()}`
      await transport.createSession(name)
      await fetchSessions()
    } catch (err) {
      console.error('Failed to create session:', err)
    }
  }, [transport, fetchSessions])

  const handleNewShell = useCallback(async () => {
    if (!activeSessionId) return

    try {
      const ptyId = await transport.spawnShell(activeSessionId)
      const conn = transport.attachPty(activeSessionId, ptyId, 80, 24)
      connectionsRef.current.push(conn)

      const newTab: PtyTab = {
        id: ptyId,
        label: `Shell ${ptyId}`,
        isAgent: false,
      }
      setPtyStates((prev) => [...prev, { tab: newTab, connection: conn }])
      setActiveTab(ptyId)
    } catch (err) {
      console.error('Failed to spawn shell:', err)
    }
  }, [activeSessionId, transport])

  const handleCloseTab = useCallback(
    (id: number) => {
      setPtyStates((prev) => {
        const idx = prev.findIndex((s) => s.tab.id === id)
        if (idx === -1) return prev

        // Close the connection
        const state = prev[idx]
        if (state.connection) {
          state.connection.close()
          connectionsRef.current = connectionsRef.current.filter(
            (c) => c !== state.connection,
          )
        }

        const next = prev.filter((_, i) => i !== idx)

        // If closing the active tab, switch to the last tab
        if (activeTab === id && next.length > 0) {
          setActiveTab(next[next.length - 1].tab.id)
        }

        return next
      })
    },
    [activeTab],
  )

  const handleResize = useCallback(
    async (cols: number, rows: number) => {
      if (!activeSessionId) return
      try {
        await transport.resizePty(activeSessionId, activeTab, cols, rows)
      } catch {
        // ignore
      }
    },
    [activeSessionId, activeTab, transport],
  )

  const activePty = ptyStates.find((s) => s.tab.id === activeTab)

  return (
    <div className="flex h-full">
      <SessionSidebar
        sessions={sessions}
        activeSessionId={activeSessionId}
        onSelectSession={setActiveSessionId}
        onNewSession={handleNewSession}
        open={layout.sidebarOpen}
        onOpenChange={(open) => setLayout({ sidebarOpen: open })}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {activeSessionId ? (
          <>
            <PtyTabs
              tabs={ptyStates.map((s) => s.tab)}
              activeTab={activeTab}
              onSelectTab={setActiveTab}
              onCloseTab={handleCloseTab}
              onNewShell={handleNewShell}
            />
            <div className="min-h-0 flex-1">
              <Allotment
                onDragEnd={(sizes) => {
                  if (sizes) setLayout({ splitSizes: sizes as number[] })
                }}
              >
                <Allotment.Pane>
                  <Terminal
                    connection={activePty?.connection ?? null}
                    onResize={handleResize}
                  />
                </Allotment.Pane>
              </Allotment>
            </div>
          </>
        ) : (
          <div className="flex flex-1 items-center justify-center">
            <div className="text-center">
              <h2 className="text-lg font-semibold text-muted-foreground">
                No session selected
              </h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Create or select a session to get started
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
