import { useCallback, useEffect, useId, useRef } from 'preact/hooks'
import {
  errorMessage,
  isAbortError,
  listPrinters,
  retryAfterHint,
  type OptionSpec,
  type Printer,
} from '../api'
import { useAuthGuard } from '../hooks/useAuthGuard'
import { usePolling } from '../hooks/usePolling'
import { AlertIcon, InfoIcon, PrinterIcon, RefreshIcon, SendIcon } from '../icons'
import {
  controlLabel,
  isNumericOption,
  optionChoices,
  optionDoc,
  sortOptionKeys,
  summarizeOptions,
} from '../options'
import { useAppDispatch, useAppState } from '../state'
import { Tooltip } from './Tooltip'
import './PrinterPanel.css'

const PRINTER_STATE_LABELS: Record<Printer['state'], string> = {
  idle: '空闲',
  printing: '打印中',
  stopped: '已停止',
  disabled: '已禁用',
}

const PRINTER_STATE_TIPS: Record<Printer['state'], string> = {
  idle: '空闲：可以接收新任务',
  printing: '打印中：正在输出任务',
  stopped: '已停止：CUPS 暂停了该队列',
  disabled: '已禁用：该队列已被禁用',
}

function OptionField({ optionKey, spec }: { optionKey: string; spec: OptionSpec }) {
  const state = useAppState()
  const dispatch = useAppDispatch()
  // 选项名的专业解释（IPP 属性名与含义）走 Tooltip：视觉上悬停展示，
  // 屏幕阅读器通过 aria-describedby 朗读，两边语义一致。
  const tipId = useId()
  const doc = optionDoc(optionKey)
  const value = state.options[optionKey] ?? ''

  return (
    <Tooltip tip={doc} describedId={tipId} className="tip--block">
      <label class="field">
        <span class="field__label">
          <span class="field__label-text">{controlLabel(optionKey)}</span>
        </span>
        {isNumericOption(spec) ? (
          <input
            type="number"
            inputMode="numeric"
            min={spec.min}
            max={spec.max}
            step={1}
            value={value}
            aria-describedby={tipId}
            onInput={(event) => {
              // 空值也要写回状态：否则 DOM 已经清空、state 里仍是旧值，
              // 确认对话框与真正提交的份数会和用户看到的不一致（视为“打印机默认”）。
              dispatch({ type: 'options/set', key: optionKey, value: event.currentTarget.value })
            }}
          />
        ) : (
          <select
            value={value}
            aria-describedby={tipId}
            onChange={(event) => {
              dispatch({ type: 'options/set', key: optionKey, value: event.currentTarget.value })
            }}
          >
            {optionChoices(optionKey, spec).map((choice) => (
              <option key={choice.value} value={choice.value}>
                {choice.label}
              </option>
            ))}
          </select>
        )}
      </label>
    </Tooltip>
  )
}

/**
 * 构造提交给服务端的选项目录：
 *   - 丢掉空值：数字输入框被清空表示「使用打印机默认」，不能把空串发给 IPP 编码器；
 *   - 丢掉打印机当前目录里不存在的键：打印机重置或读取选项目录失败后，
 *     残留的旧键会被服务端判为 `invalid_controls`（400）而整单被拒。
 */
export function payloadOptions(
  printer: Printer,
  values: Record<string, string>,
): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [key, value] of Object.entries(values)) {
    if (value !== '' && key in printer.options) {
      result[key] = value
    }
  }
  return result
}

export function PrinterPanel() {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const { printers, selectedPrinterId, options, upload } = state
  const abortRef = useRef<AbortController | null>(null)

  // 组件卸载（例如退出登录）时中断手动刷新，避免响应落到已经重置的会话状态里。
  useEffect(() => {
    return () => {
      abortRef.current?.abort()
      abortRef.current = null
    }
  }, [])

  const load = useCallback(
    async (signal: AbortSignal, bypassCache: boolean): Promise<void> => {
      const items = await listPrinters({ refresh: bypassCache, signal })
      dispatch({ type: 'printers/loaded', printers: items })
    },
    [dispatch],
  )

  usePolling(
    async (signal) => {
      dispatch({ type: 'printers/loading' })
      try {
        await load(signal, false)
      } catch (error) {
        if (isAbortError(error)) {
          return
        }
        if (authGuard(error)) {
          return
        }
        dispatch({
          type: 'printers/error',
          message: `加载失败：${errorMessage(error)}${retryAfterHint(error)}`,
        })
        throw error
      }
    },
    { intervalMs: 5000, enabled: state.token !== '' },
  )

  async function handleManualRefresh(): Promise<void> {
    if (printers.refreshing) {
      return
    }
    abortRef.current?.abort()
    const controller = new AbortController()
    abortRef.current = controller
    dispatch({ type: 'printers/refreshing', value: true })
    try {
      await load(controller.signal, true)
    } catch (error) {
      if (isAbortError(error) || controller.signal.aborted) {
        return
      }
      if (authGuard(error)) {
        return
      }
      dispatch({
        type: 'printers/error',
        message: `加载失败：${errorMessage(error)}${retryAfterHint(error)}`,
      })
    } finally {
      // 只有当前请求才允许清掉 spinner：被后发请求取代的旧请求结束时不能提前收起。
      if (abortRef.current === controller) {
        abortRef.current = null
        dispatch({ type: 'printers/refreshing', value: false })
      }
    }
  }

  const selected = printers.items.find((printer) => printer.id === selectedPrinterId) ?? null
  const canPrint =
    upload.result !== null &&
    selected !== null &&
    selected.accepting_jobs &&
    !state.print.submitting

  function openConfirmation(): void {
    if (upload.result === null || selected === null) {
      return
    }
    const chosen = payloadOptions(selected, options)
    dispatch({
      type: 'print/confirm',
      pending: {
        payload: { file_id: upload.result.id, printer_id: selected.id, options: chosen },
        printerName: selected.name,
        fileName: upload.result.name,
        summary: summarizeOptions(selected, chosen).map((row) => ({
          key: row.key,
          label: row.label,
          value: row.value,
        })),
      },
    })
  }

  const optionKeys = selected !== null ? sortOptionKeys(Object.keys(selected.options)) : []

  return (
    <section class="card printer-panel" aria-labelledby="printer-panel-title">
      <div class="card__header">
        <span class="card__icon card__icon--print" aria-hidden="true">
          <PrinterIcon size={18} />
        </span>
        <h2 id="printer-panel-title">打印</h2>
        <button
          type="button"
          class="ghost ghost--sm ghost--icon tip"
          data-tip="刷新打印机列表（跳过服务端缓存）"
          aria-label="刷新打印机列表（跳过服务端缓存）"
          onClick={() => void handleManualRefresh()}
          disabled={printers.refreshing}
        >
          <RefreshIcon size={14} className={printers.refreshing ? 'refresh-spin' : undefined} />
        </button>
      </div>

      <div class="printer-panel__body" aria-busy={printers.status === 'loading'}>
        {/* 刷新失败时如果已经拿到过打印机列表，就只在顶部提示错误：
            整块替换成错误页会让选择框、选项和「提交打印」一起消失，
            用户明明还能用却像服务挂了。排查建议收进 Tooltip，版面只留结论。 */}
        {printers.error !== null ? (
          <div class="inline-error" role="alert">
            <AlertIcon size={16} />
            <Tooltip tip="请确认容器内 CUPS 服务已启动（可用 JUST_PRINT_CUPS_PDF=1 添加 CUPS-PDF 调试打印机）；服务会自动重试。">
              <span>{printers.error}</span>
            </Tooltip>
          </div>
        ) : null}

        {/* 从未拿到过打印机列表、也没有错误时，才提示「未发现打印机」。 */}
        {printers.items.length === 0 && printers.error === null ? (
          <div class="inline-hint">
            <RefreshIcon size={15} className="refresh-spin" />
            <Tooltip tip="请先在 CUPS 中配置打印机（容器内可用 JUST_PRINT_CUPS_PDF=1 添加 CUPS-PDF 调试打印机）；服务会自动重试。">
              <span>未发现打印机，自动重试中</span>
            </Tooltip>
          </div>
        ) : null}

        {printers.items.length > 0 ? (
          <>
            <label class="field">
              <span class="field__label">打印机</span>
              <select
                value={selectedPrinterId}
                onChange={(event) =>
                  dispatch({ type: 'printer/select', id: event.currentTarget.value })
                }
              >
                {printers.items.map((printer) => (
                  <option key={printer.id} value={printer.id}>
                    {printer.name}
                    {!printer.accepting_jobs ? '（不接收任务）' : ''}
                  </option>
                ))}
              </select>
            </label>

            {selected !== null ? (
              <div class="printer-panel__summary">
                <Tooltip tip={PRINTER_STATE_TIPS[selected.state]}>
                  <span class={`printer-state printer-state--${selected.state}`}>
                    {PRINTER_STATE_LABELS[selected.state]}
                  </span>
                </Tooltip>
                {!selected.accepting_jobs ? (
                  <Tooltip tip="CUPS 已暂停该队列接收新任务，请在 CUPS 中恢复">
                    <span class="printer-state printer-state--disabled">不接收任务</span>
                  </Tooltip>
                ) : null}
                {selected.make_and_model !== null ? (
                  <Tooltip tip="打印机型号（CUPS make_and_model）">
                    <span class="printer-panel__detail">{selected.make_and_model}</span>
                  </Tooltip>
                ) : null}
                {selected.location !== null ? (
                  <Tooltip tip="打印机位置（CUPS location）">
                    <span class="printer-panel__detail">位置：{selected.location}</span>
                  </Tooltip>
                ) : null}
                {optionKeys.length > 0 ? (
                  <Tooltip tip="该打印机通过 IPP 暴露的可调参数数量">
                    <span class="printer-panel__detail">{optionKeys.length} 个可调选项</span>
                  </Tooltip>
                ) : null}
              </div>
            ) : null}

            {selected?.options_error !== null && selected?.options_error !== undefined ? (
              <div class="inline-hint inline-hint--warning">
                <InfoIcon size={15} />
                <Tooltip tip={selected.options_error}>
                  <span>选项目录读取失败，使用默认设置</span>
                </Tooltip>
              </div>
            ) : null}

            {selected !== null && optionKeys.length === 0 ? (
              <div class="inline-hint">
                <InfoIcon size={15} />
                <span>无可调选项，按默认设置打印</span>
              </div>
            ) : null}

            {selected !== null && optionKeys.length > 0 ? (
              <div class="printer-panel__controls">
                {optionKeys.map((key) => (
                  <OptionField key={key} optionKey={key} spec={selected.options[key]} />
                ))}
              </div>
            ) : null}

            {selected !== null && optionKeys.length > 0 ? (
              <button
                type="button"
                class="ghost ghost--sm tip printer-panel__reset"
                data-tip="恢复推荐设置（A4 · 双面长边 · 最高分辨率）"
                aria-label="恢复推荐设置（A4 · 双面长边 · 最高分辨率）"
                onClick={() => dispatch({ type: 'options/reset' })}
              >
                <RefreshIcon size={14} />
                重置
              </button>
            ) : null}

            {upload.result === null ? (
              <div class="inline-hint">
                <InfoIcon size={15} />
                <span>请先上传文档</span>
              </div>
            ) : null}

            {selected !== null && !selected.accepting_jobs ? (
              <div class="inline-error" role="alert">
                <AlertIcon size={16} />
                <span>该打印机不接收新任务，请在 CUPS 中恢复</span>
              </div>
            ) : null}

            <button
              type="button"
              class="primary printer-panel__submit"
              disabled={!canPrint}
              onClick={openConfirmation}
            >
              <SendIcon size={16} />
              提交打印
            </button>
          </>
        ) : null}
      </div>
    </section>
  )
}
