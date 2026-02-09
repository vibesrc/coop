import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet'
import { cn } from '@/lib/utils'
import { Plus, Menu, Circle, Users } from 'lucide-react'
import type { Session } from '@/transport'

interface SessionSidebarProps {
  sessions: Session[]
  activeSessionId: string | null
  onSelectSession: (id: string) => void
  onNewSession: () => void
  open: boolean
  onOpenChange: (open: boolean) => void
}

function SessionList({
  sessions,
  activeSessionId,
  onSelectSession,
  onNewSession,
}: Omit<SessionSidebarProps, 'open' | 'onOpenChange'>) {
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-semibold">Sessions</span>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onNewSession}
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>
      <ScrollArea className="flex-1">
        <div className="space-y-0.5 p-1">
          {sessions.length === 0 && (
            <p className="px-3 py-6 text-center text-xs text-muted-foreground">
              No sessions
            </p>
          )}
          {sessions.map((session) => (
            <button
              key={session.id}
              onClick={() => onSelectSession(session.id)}
              className={cn(
                'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm transition-colors',
                activeSessionId === session.id
                  ? 'bg-accent text-accent-foreground'
                  : 'text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground',
              )}
            >
              <Circle
                className={cn(
                  'h-2 w-2 shrink-0 fill-current',
                  session.status === 'running'
                    ? 'text-green-500'
                    : 'text-muted-foreground',
                )}
              />
              <span className="flex-1 truncate text-left">{session.name}</span>
              {session.clients > 0 && (
                <span className="flex items-center gap-0.5 text-xs text-muted-foreground">
                  <Users className="h-3 w-3" />
                  {session.clients}
                </span>
              )}
            </button>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}

/** Desktop sidebar */
export function SessionSidebar(props: SessionSidebarProps) {
  return (
    <>
      {/* Desktop: always-visible sidebar */}
      <div
        className={cn(
          'hidden h-full w-56 shrink-0 border-r border-border bg-card md:block',
          !props.open && 'md:hidden',
        )}
      >
        <SessionList
          sessions={props.sessions}
          activeSessionId={props.activeSessionId}
          onSelectSession={props.onSelectSession}
          onNewSession={props.onNewSession}
        />
      </div>

      {/* Mobile: slide-out drawer */}
      <Sheet open={props.open} onOpenChange={props.onOpenChange}>
        <SheetTrigger asChild className="md:hidden">
          <Button
            variant="ghost"
            size="icon"
            className="absolute left-2 top-2 z-10 h-8 w-8 md:hidden"
          >
            <Menu className="h-4 w-4" />
          </Button>
        </SheetTrigger>
        <SheetContent side="left" className="w-56 p-0">
          <SheetHeader className="sr-only">
            <SheetTitle>Sessions</SheetTitle>
          </SheetHeader>
          <SessionList
            sessions={props.sessions}
            activeSessionId={props.activeSessionId}
            onSelectSession={(id) => {
              props.onSelectSession(id)
              props.onOpenChange(false)
            }}
            onNewSession={() => {
              props.onNewSession()
              props.onOpenChange(false)
            }}
          />
        </SheetContent>
      </Sheet>
    </>
  )
}
