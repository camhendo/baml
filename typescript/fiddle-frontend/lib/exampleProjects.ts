import type { EditorFile } from '@/app/actions'

// export type ParserDBFunctionTestModel = Pick<ParserDatabase['functions'][0], 'name' | 'test_cases'>;

export type BAMLProject = {
  id: string
  name: string
  description: string
  files: EditorFile[]
  filePath?: string
  // functionsWithTests: ParserDBFunctionTestModel[];
  testRunOutput?: any
}
