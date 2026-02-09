import type { Transport, Session, PtyConnection } from './transport'

export class HttpTransport implements Transport {
  private baseUrl: string
  private wsBaseUrl: string
  private token: string | null

  constructor(token?: string | null, baseUrl?: string) {
    const loc = window.location
    this.baseUrl = baseUrl ?? `${loc.protocol}//${loc.host}`
    const wsProto = loc.protocol === 'https:' ? 'wss:' : 'ws:'
    this.wsBaseUrl = baseUrl
      ? baseUrl.replace(/^http/, 'ws')
      : `${wsProto}//${loc.host}`
    this.token = token ?? null
  }

  private headers(): Record<string, string> {
    const h: Record<string, string> = { 'Content-Type': 'application/json' }
    if (this.token) h['Authorization'] = `Bearer ${this.token}`
    return h
  }

  async listSessions(): Promise<Session[]> {
    const res = await fetch(`${this.baseUrl}/api/sessions`, {
      headers: this.headers(),
    })
    if (!res.ok) throw new Error(`Failed to list sessions: ${res.status}`)
    return res.json()
  }

  async createSession(name: string): Promise<string> {
    const res = await fetch(`${this.baseUrl}/api/sessions`, {
      method: 'POST',
      headers: this.headers(),
      body: JSON.stringify({ name }),
    })
    if (!res.ok) throw new Error(`Failed to create session: ${res.status}`)
    const data = await res.json()
    return data.id
  }

  async killSession(id: string): Promise<void> {
    const res = await fetch(`${this.baseUrl}/api/sessions/${id}`, {
      method: 'DELETE',
      headers: this.headers(),
    })
    if (!res.ok) throw new Error(`Failed to kill session: ${res.status}`)
  }

  async spawnShell(sessionId: string): Promise<number> {
    const res = await fetch(
      `${this.baseUrl}/api/sessions/${sessionId}/shell`,
      {
        method: 'POST',
        headers: this.headers(),
      },
    )
    if (!res.ok) throw new Error(`Failed to spawn shell: ${res.status}`)
    const data = await res.json()
    return data.pty_id
  }

  attachPty(sessionId: string, ptyId: number, cols: number, rows: number): PtyConnection {
    const params = new URLSearchParams({
      pty: String(ptyId),
      cols: String(cols),
      rows: String(rows),
    })
    if (this.token) params.set('token', this.token)

    const url = `${this.wsBaseUrl}/api/sessions/${sessionId}/pty?${params}`
    const ws = new WebSocket(url)
    ws.binaryType = 'arraybuffer'

    const dataCbs: Array<(data: string | ArrayBuffer) => void> = []
    const closeCbs: Array<() => void> = []

    ws.onmessage = (e) => {
      for (const cb of dataCbs) cb(e.data)
    }
    ws.onclose = () => {
      for (const cb of closeCbs) cb()
    }

    return {
      send(data) {
        if (ws.readyState === WebSocket.OPEN) ws.send(data)
      },
      close() {
        ws.close()
      },
      onData(cb) {
        dataCbs.push(cb)
      },
      onClose(cb) {
        closeCbs.push(cb)
      },
    }
  }

  async resizePty(
    sessionId: string,
    ptyId: number,
    cols: number,
    rows: number,
  ): Promise<void> {
    const res = await fetch(
      `${this.baseUrl}/api/sessions/${sessionId}/pty/${ptyId}/resize`,
      {
        method: 'POST',
        headers: this.headers(),
        body: JSON.stringify({ cols, rows }),
      },
    )
    if (!res.ok) throw new Error(`Failed to resize PTY: ${res.status}`)
  }

  dispose(): void {
    // Individual PtyConnections should be closed by the caller.
  }
}
