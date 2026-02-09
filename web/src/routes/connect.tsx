import { createFileRoute } from '@tanstack/react-router'
import { useMemo } from 'react'
import { AppLayout } from '@/components/app-layout'
import { WebRtcTransport } from '@/transport'

export const Route = createFileRoute('/connect')({
  component: ConnectPage,
})

function ConnectPage() {
  // TODO: parse signaling info from URL fragment and initialize WebRTC
  const transport = useMemo(() => new WebRtcTransport(), [])

  return <AppLayout transport={transport} />
}
