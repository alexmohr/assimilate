// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { mount } from '@vue/test-utils'
import type { TooltipItem } from 'chart.js'
import { useBytesLineChartOptions } from './useBytesLineChartOptions'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
}))

function mountOptions(): ReturnType<typeof useBytesLineChartOptions> {
  let result: ReturnType<typeof useBytesLineChartOptions> | undefined
  mount(
    defineComponent({
      setup() {
        result = useBytesLineChartOptions()
        return () => h('div')
      },
    }),
  )
  return result!
}

function tooltipItem(y: number): TooltipItem<'line'> {
  return {
    dataset: { label: 'Original' },
    parsed: { y },
  } as unknown as TooltipItem<'line'>
}

describe('useBytesLineChartOptions', () => {
  it('shows a legend when showLegend is true', () => {
    const { bytesLineOptions } = mountOptions()
    const options = bytesLineOptions(true)

    expect(options.plugins?.legend).toMatchObject({ display: true })
  })

  it('hides the legend when showLegend is false', () => {
    const { bytesLineOptions } = mountOptions()
    const options = bytesLineOptions(false)

    expect(options.plugins?.legend).toEqual({ display: false })
  })

  it('formats the tooltip label with the dataset name and byte value', () => {
    const { bytesLineOptions } = mountOptions()
    const options = bytesLineOptions(true)

    const label = options.plugins?.tooltip?.callbacks?.label
    expect(label?.call(undefined, tooltipItem(1024))).toBe('Original: 1024B')
  })

  it('formats the y-axis tick callback with formatBytes', () => {
    const { bytesLineOptions } = mountOptions()
    const options = bytesLineOptions(true)

    const scaleY = options.scales?.y as { ticks?: { callback?: (v: string | number) => string } }
    expect(scaleY.ticks?.callback?.(2048)).toBe('2048B')
  })
})
