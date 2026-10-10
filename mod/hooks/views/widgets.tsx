import type { BoxProps, ButtonProps, ElementConstructor, TextProps } from 'claude-code'

import type { Line, Segment } from '../state'

export type Ui = {
  Box: ElementConstructor<BoxProps>
  Text: ElementConstructor<TextProps>
  Button: ElementConstructor<ButtonProps>
}

export function Piece({ ui, segment }: { ui: Ui; segment: Segment }) {
  const { Text } = ui
  return (
    <Text color={segment.color} dimColor={segment.dim} bold={segment.bold}>
      {segment.text}
    </Text>
  )
}

export function Row({ ui, line, width, onWhy }: { ui: Ui; line: Line; width: number; onWhy: (key: string) => void }) {
  const { Box, Text, Button } = ui
  const text = (
    <Text wrap={line.wrap === true ? 'wrap' : 'truncate-end'}>
      {line.segments.map(segment => (
        <Piece ui={ui} segment={segment} />
      ))}
    </Text>
  )
  const why = line.why
  if (why === undefined) {
    return (
      <Box key={line.key} width={width} paddingLeft={line.indent ?? 0}>
        {text}
      </Box>
    )
  }
  return (
    <Box key={line.key} width={width} justifyContent="space-between">
      {text}
      <Button key={`why-${why}`} plain dimColor label="?" onPress={() => onWhy(why)} />
    </Box>
  )
}
