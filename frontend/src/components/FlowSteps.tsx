import { CheckIcon } from '../icons'

export type StepState = 'done' | 'active' | 'todo'

interface FlowStepsProps {
  steps: { label: string; state: StepState }[]
}

/** 主流程步骤条：上传 → 打印 → 任务状态，随页面状态推进。 */
export function FlowSteps({ steps }: FlowStepsProps) {
  return (
    <ol class="flow" aria-label="打印流程">
      {steps.map((step, index) => (
        <li
          key={step.label}
          class={`flow-step flow-${step.state}`}
          aria-current={step.state === 'active' ? 'step' : undefined}
        >
          <span class="flow-node" aria-hidden={step.state === 'done' ? undefined : true}>
            {step.state === 'done' ? <CheckIcon size={14} /> : index + 1}
          </span>
          <span class="flow-label">{step.label}</span>
        </li>
      ))}
    </ol>
  )
}
