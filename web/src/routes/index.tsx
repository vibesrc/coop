import { createFileRoute } from '@tanstack/react-router'
import { useMemo } from 'react'
import { AppLayout } from '@/components/app-layout'
import { HttpTransport } from '@/transport'
import { initToken } from '@/lib/auth'

export const Route = createFileRoute('/')({
  component: IndexPage,
})

function IndexPage() {
  const transport = useMemo(() => {
    const token = initToken()
    return new HttpTransport(token)
  }, [])

  return <AppLayout transport={transport} />
}
