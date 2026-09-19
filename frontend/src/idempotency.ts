/* =============================================================================
 * 打印幂等键管理。
 *
 * 规则：
 *   - 每个“逻辑打印尝试”对应一个 `Idempotency-Key`（UUID v4）；
 *   - 同一负载（file_id + printer_id + options）在重试时复用同一把键，
 *     这样 503/504/网络抖动后的重试不会重复出纸；
 *   - 提交成功后把尝试标记为 settled，下一次打印生成新键；
 *   - 负载变化（换文件、换打印机、改选项）视为新的打印尝试，生成新键。
 * ========================================================================== */

import type { PrintPayload } from './api'

export interface PrintAttempt {
  /** 负载指纹，用于判断是否仍是同一次打印。 */
  fingerprint: string
  /** 发送给服务端的 Idempotency-Key。 */
  key: string
  /** 已确认成功（含服务端回放）；settled 的键不再复用。 */
  settled: boolean
}

export function defaultCreateKey(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID()
  }
  return `jp-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

/** 稳定负载指纹：键排序后序列化，避免键顺序影响判断。 */
export function printFingerprint(payload: PrintPayload): string {
  const options = Object.keys(payload.options)
    .sort()
    .map((key) => [key, payload.options[key]] as const)
  return JSON.stringify({ file_id: payload.file_id, printer_id: payload.printer_id, options })
}

export interface ResolvedAttempt {
  attempt: PrintAttempt
  /** true 表示复用了已有键（属于同一次打印尝试的重试）。 */
  reused: boolean
}

export function resolveAttempt(
  previous: PrintAttempt | null,
  fingerprint: string,
  createKey: () => string = defaultCreateKey,
): ResolvedAttempt {
  if (previous !== null && !previous.settled && previous.fingerprint === fingerprint) {
    return { attempt: previous, reused: true }
  }
  return {
    attempt: { fingerprint, key: createKey(), settled: false },
    reused: false,
  }
}
