export interface Session {
  id: string
  name: string
  status: 'running' | 'exited'
  clients: number
  created_at: string
}

export interface PtyInfo {
  id: number
  label: string
}

export interface Transport {
  /** List all active sessions */
  listSessions(): Promise<Session[]>

  /** Create a new session, returns session ID */
  createSession(name: string): Promise<string>

  /** Kill/delete a session */
  killSession(id: string): Promise<void>

  /** Spawn a new shell PTY in a session, returns PTY id */
  spawnShell(sessionId: string): Promise<number>

  /** Attach to a PTY and get a bidirectional connection */
  attachPty(
    sessionId: string,
    ptyId: number,
    cols: number,
    rows: number,
  ): PtyConnection

  /** Resize an existing PTY */
  resizePty(sessionId: string, ptyId: number, cols: number, rows: number): Promise<void>

  /** Dispose of the transport and any open connections */
  dispose(): void
}

export interface PtyConnection {
  send(data: string | ArrayBuffer): void
  close(): void
  onData(cb: (data: string | ArrayBuffer) => void): void
  onClose(cb: () => void): void
}
