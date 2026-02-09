import type { Transport, Session, PtyConnection } from './transport'

/**
 * WebRTC DataChannel transport. Uses an RTCDataChannel to tunnel
 * API calls and PTY streams over a peer-to-peer connection.
 *
 * TODO: implement signaling and DataChannel setup
 */
export class WebRtcTransport implements Transport {
  async listSessions(): Promise<Session[]> {
    throw new Error('WebRTC transport not yet implemented')
  }

  async createSession(_name: string): Promise<string> {
    throw new Error('WebRTC transport not yet implemented')
  }

  async killSession(_id: string): Promise<void> {
    throw new Error('WebRTC transport not yet implemented')
  }

  async spawnShell(_sessionId: string): Promise<number> {
    throw new Error('WebRTC transport not yet implemented')
  }

  attachPty(
    _sessionId: string,
    _ptyId: number,
    _cols: number,
    _rows: number,
  ): PtyConnection {
    throw new Error('WebRTC transport not yet implemented')
  }

  async resizePty(
    _sessionId: string,
    _ptyId: number,
    _cols: number,
    _rows: number,
  ): Promise<void> {
    throw new Error('WebRTC transport not yet implemented')
  }

  dispose(): void {
    // Nothing to clean up yet
  }
}
