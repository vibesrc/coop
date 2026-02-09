import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Plus, X, Bot, TerminalSquare } from 'lucide-react'

export interface PtyTab {
  id: number
  label: string
  isAgent: boolean
}

interface PtyTabsProps {
  tabs: PtyTab[]
  activeTab: number
  onSelectTab: (id: number) => void
  onCloseTab: (id: number) => void
  onNewShell: () => void
}

export function PtyTabs({
  tabs,
  activeTab,
  onSelectTab,
  onCloseTab,
  onNewShell,
}: PtyTabsProps) {
  return (
    <div className="flex h-9 items-center gap-0.5 border-b border-border bg-background px-1">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onSelectTab(tab.id)}
          className={cn(
            'group relative flex h-7 items-center gap-1.5 rounded-sm px-2.5 text-xs transition-colors',
            activeTab === tab.id
              ? 'bg-accent text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground',
          )}
        >
          {tab.isAgent ? (
            <Bot className="h-3.5 w-3.5 shrink-0" />
          ) : (
            <TerminalSquare className="h-3.5 w-3.5 shrink-0" />
          )}
          <span className="max-w-[100px] truncate">{tab.label}</span>
          {!tab.isAgent && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                onCloseTab(tab.id)
              }}
              className={cn(
                'ml-0.5 inline-flex h-4 w-4 items-center justify-center rounded-sm opacity-0 transition-opacity hover:bg-muted',
                'group-hover:opacity-100',
                activeTab === tab.id && 'opacity-60',
              )}
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </button>
      ))}

      <Button
        variant="ghost"
        size="icon"
        className="ml-0.5 h-6 w-6"
        onClick={onNewShell}
      >
        <Plus className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}
