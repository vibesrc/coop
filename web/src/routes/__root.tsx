import { createRootRoute, Outlet } from '@tanstack/react-router'
import { TooltipProvider } from '@/components/ui/tooltip'

export const Route = createRootRoute({
  component: () => (
    <div className="dark h-screen w-screen overflow-hidden bg-background text-foreground">
      <TooltipProvider>
        <Outlet />
      </TooltipProvider>
    </div>
  ),
})
