import { CheckIcon } from '../icons'
import type { StepState } from '../state'
import './FlowSteps.css'

interface FlowStepsProps {
  steps: { label: string; state: StepState }[]
}

/** 主流程步骤条：上传 → 打印 → 任务状态，状态由全局 reducer 派生。 */
export function FlowSteps({ steps }: FlowStepsProps) {
  return (
    <ol class="flow" aria-label="打印流程">
      {steps.map((step, index) => (
        <li
          key={step.label}
          class={`flow__step flow__step--${step.state}`}
          aria-current={step.state === 'active' ? 'step' : undefined}
        >
          <span class="flow__node" aria-hidden="true">
            {step.state === 'done' ? <CheckIcon size={14} /> : index + 1}
          </span>
          <span class="flow__label">
            {step.label}
            <span class="visually-hidden">
              {step.state === 'done'
                ? '（已完成）'
                : step.state === 'active'
                  ? '（当前步骤）'
                  : '（未开始）'}
            </span>
          </span>
        </li>
      ))}
    </ol>
  )
}
