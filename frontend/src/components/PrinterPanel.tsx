import { useEffect, useRef, useState } from 'preact/hooks'
import type { JSX } from 'preact'
import {
  type CapabilityView,
  type Printer,
  type UploadResult,
  errorMessage,
  isUnauthorized,
  listPrinters,
  submitPrint,
} from '../api'

interface PrinterPanelProps {
  upload: UploadResult | null
  onJobSubmitted: (jobId: string) => void
  onAuthFailure: () => void
  onNotice: (message: string | null) => void
}

function defaultsFor(printer: Printer): Record<string, string> {
  const result: Record<string, string> = {}
  const capabilities = printer.capabilities ?? {}
  for (const [key, cap] of Object.entries(capabilities)) {
    if (cap.default !== null && cap.default !== undefined) {
      result[key] = cap.default
    } else if (cap.kind === 'enumerated' && cap.values && cap.values.length > 0) {
      result[key] = cap.values[0]
    } else if (cap.kind === 'range' && cap.min !== undefined) {
      result[key] = String(cap.min)
    }
  }
  return result
}

export function PrinterPanel({
  upload,
  onJobSubmitted,
  onAuthFailure,
  onNotice,
}: PrinterPanelProps) {
  const [printers, setPrinters] = useState<Printer[]>([])
  const [selectedId, setSelectedId] = useState('')
  const [controls, setControls] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    void refresh()
    const interval = window.setInterval(() => void refresh(), 5000)
    return () => {
      mountedRef.current = false
      window.clearInterval(interval)
    }
  }, [])

  async function refresh(): Promise<void> {
    try {
      const result = await listPrinters()
      if (!mountedRef.current) {
        return
      }
      setPrinters(result.printers)
      setSelectedId((previous) => {
        if (previous && result.printers.some((printer) => printer.id === previous)) {
          return previous
        }
        const first =
          result.printers.find(
            (printer) =>
              printer.pdf_supported ||
              printer.postscript_supported ||
              printer.pcl_supported,
          ) ?? result.printers[0]
        return first ? first.id : ''
      })
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      if (mountedRef.current) {
        setError(`打印机列表加载失败：${errorMessage(requestError)}`)
      }
    }
  }

  function selectPrinter(id: string): void {
    setSelectedId(id)
    const printer = printers.find((candidate) => candidate.id === id)
    setControls(printer ? defaultsFor(printer) : {})
  }

  function setControl(key: string, value: string): void {
    setControls((previous) => ({ ...previous, [key]: value }))
  }

  function renderControl(key: string, cap: CapabilityView): JSX.Element | null {
    if (cap.kind === 'enumerated') {
      const options = cap.values ?? []
      if (options.length === 0) {
        return null
      }
      return (
        <select
          value={controls[key] ?? ''}
          onInput={(event) =>
            setControl(key, (event.target as HTMLSelectElement).value)}
        >
          {options.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      )
    }
    const min = cap.min ?? 0
    const max = cap.max ?? 100
    return (
      <input
        type="number"
        min={min}
        max={max}
        value={controls[key] ?? ''}
        onInput={(event) =>
          setControl(key, (event.target as HTMLInputElement).value)}
      />
    )
  }

  const selected = printers.find((printer) => printer.id === selectedId) ?? null
  const canPrint =
    upload !== null &&
    selected !== null &&
    (selected.pdf_supported ||
      selected.postscript_supported ||
      selected.pcl_supported) &&
    selected.capabilities !== null &&
    !busy

  async function handlePrint(): Promise<void> {
    if (!upload || !selected || busy) {
      return
    }
    if (selected.capabilities === null) {
      setError('打印机能力尚未加载，请稍候')
      return
    }
    if (
      !selected.pdf_supported &&
      !selected.pcl_supported &&
      !selected.postscript_supported
    ) {
      setError('打印机不支持 PDF / PCL / PostScript 输出')
      return
    }
    setBusy(true)
    setError(null)
    onNotice(null)
    try {
      const result = await submitPrint({
        file_id: upload.id,
        printer_id: selected.id,
        controls,
      })
      onJobSubmitted(result.job_id)
      onNotice('打印任务已提交')
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      setError(`提交失败：${errorMessage(requestError)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <section class="card">
      <h2>打印</h2>
      {printers.length === 0 ? (
        <p class="muted">
          未发现打印机。请确认设备已通过 /dev/usb/lp* 映射到容器，服务会每 5 秒自动重试。
        </p>
      ) : (
        <>
          <label class="field">
            <span>打印机</span>
            <select
              value={selectedId}
              onInput={(event) => selectPrinter((event.target as HTMLSelectElement).value)}
            >
              {printers.map((printer) => (
                <option key={printer.id} value={printer.id}>
                  {printer.name}
                  {printer.serial ? `（${printer.serial}）` : ''}
                </option>
              ))}
            </select>
          </label>
          {selected &&
          !selected.pdf_supported &&
          !selected.pcl_supported &&
          !selected.postscript_supported ? (
            <p class="error">该打印机不支持 PDF / PCL / PostScript 输出，无法打印。</p>
          ) : null}
          {selected && !selected.pdf_supported && selected.pcl_supported ? (
            <p class="muted">该打印机不支持 PDF，将使用 PCL 打印。</p>
          ) : null}
          {selected &&
          !selected.pdf_supported &&
          !selected.pcl_supported &&
          selected.postscript_supported ? (
            <p class="muted">该打印机不支持 PDF，将使用 PostScript 打印。</p>
          ) : null}
          {selected && selected.capabilities === null ? (
            <p class="muted">打印机能力加载中…</p>
          ) : null}
          {selected && selected.capabilities ? (
            <div class="controls">
              {Object.entries(selected.capabilities).map(([key, cap]) => (
                <label class="field" key={key}>
                  <span>{key}</span>
                  {renderControl(key, cap)}
                </label>
              ))}
            </div>
          ) : null}
          {!upload ? (
            <p class="hint">请先上传文档后再打印。</p>
          ) : null}
          <button
            type="button"
            class="primary"
            disabled={!canPrint}
            onClick={() => void handlePrint()}
          >
            {busy ? '提交中…' : '提交打印'}
          </button>
          {error ? <p class="error">{error}</p> : null}
        </>
      )}
    </section>
  )
}
