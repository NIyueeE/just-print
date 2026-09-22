import type { ComponentChildren } from 'preact'
import './Tooltip.css'

interface TooltipProps {
  /** 悬停/聚焦时展示的解释文本（专业名词、缩写、完整取值等）。 */
  tip: string
  /** 传入时渲染对应的可访问描述节点，供控件 `aria-describedby` 引用。 */
  describedId?: string
  /** 附加类名（如包裹整列字段时占用 grid 单元）。 */
  className?: string
  children: ComponentChildren
}

/**
 * 悬停提示（Hover Tooltip）：
 *   - 纯 CSS 驱动，无 JS 定位；鼠标悬停或键盘聚焦（:focus-within）时展开；
 *   - 文本同时以 `data-tip` 暴露，配合 Tooltip.css 的气泡样式；
 *   - 传入 `describedId` 时额外渲染视觉隐藏的说明节点，屏幕阅读器聚焦
 *     关联控件后即可朗读，与视觉提示语义一致。
 *
 * 用法一（包裹行内文本，如字段标签、徽章）：
 *   <Tooltip tip="IPP media · 纸张大小">纸张大小</Tooltip>
 *
 * 用法二（包裹整列字段，解释挂到控件上）：
 *   <Tooltip tip={doc} describedId={tipId}>
 *     <label class="field">…</label>
 *   </Tooltip>
 */
export function Tooltip({ tip, describedId, className, children }: TooltipProps) {
  const classes = className === undefined ? 'tip' : `tip ${className}`
  return (
    <span class={classes} data-tip={tip}>
      {children}
      {describedId !== undefined ? (
        <span class="visually-hidden" id={describedId}>
          {tip}
        </span>
      ) : null}
    </span>
  )
}
