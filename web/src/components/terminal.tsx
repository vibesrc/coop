import { useEffect, useRef } from 'react'
import { Terminal as XTerm } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import type { PtyConnection } from '@/transport'

interface TerminalProps {
  connection: PtyConnection | null
  onResize?: (cols: number, rows: number) => void
}

export function Terminal({ connection, onResize }: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const xtermRef = useRef<XTerm | null>(null)
  const fitRef = useRef<FitAddon | null>(null)

  // Initialize xterm once
  useEffect(() => {
    if (!containerRef.current) return

    const term = new XTerm({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: "'JetBrains Mono', 'Cascadia Code', 'Fira Code', Menlo, monospace",
      theme: {
        background: '#09090b',
        foreground: '#fafafa',
        cursor: '#fafafa',
        selectionBackground: '#27272a',
        black: '#09090b',
        red: '#ef4444',
        green: '#22c55e',
        yellow: '#eab308',
        blue: '#3b82f6',
        magenta: '#a855f7',
        cyan: '#06b6d4',
        white: '#fafafa',
        brightBlack: '#71717a',
        brightRed: '#f87171',
        brightGreen: '#4ade80',
        brightYellow: '#facc15',
        brightBlue: '#60a5fa',
        brightMagenta: '#c084fc',
        brightCyan: '#22d3ee',
        brightWhite: '#ffffff',
      },
    })

    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(containerRef.current)

    // Small delay to ensure the container has dimensions
    requestAnimationFrame(() => {
      fit.fit()
    })

    xtermRef.current = term
    fitRef.current = fit

    // Observe container resizes
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        fit.fit()
      })
    })
    observer.observe(containerRef.current)

    return () => {
      observer.disconnect()
      term.dispose()
      xtermRef.current = null
      fitRef.current = null
    }
  }, [])

  // Wire up connection
  useEffect(() => {
    const term = xtermRef.current
    if (!term || !connection) return

    // Send terminal size on connect
    if (fitRef.current) {
      fitRef.current.fit()
    }

    // PTY -> terminal
    connection.onData((data) => {
      if (typeof data === 'string') {
        term.write(data)
      } else {
        term.write(new Uint8Array(data))
      }
    })

    connection.onClose(() => {
      term.write('\r\n\x1b[2m[connection closed]\x1b[0m\r\n')
    })

    // terminal -> PTY
    const dataDisposable = term.onData((data) => {
      connection.send(data)
    })

    return () => {
      dataDisposable.dispose()
    }
  }, [connection])

  // Notify parent of resize events
  useEffect(() => {
    const term = xtermRef.current
    if (!term || !onResize) return

    const resizeDisposable = term.onResize(({ cols, rows }) => {
      onResize(cols, rows)
    })

    return () => {
      resizeDisposable.dispose()
    }
  }, [onResize])

  return (
    <div
      ref={containerRef}
      className="h-full w-full overflow-hidden"
      style={{ backgroundColor: '#09090b' }}
    />
  )
}
