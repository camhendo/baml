import BamlWasm, { WasmRuntime, type WasmDiagnosticError } from '@gloo-ai/baml-schema-wasm-node'
import { access, mkdir, open, readdir, readFile, rename, rm, writeFile } from 'fs/promises'
import path from 'path'
import { type Diagnostic, DiagnosticSeverity, Position, LocationLink, Hover } from 'vscode-languageserver'
import { TextDocument } from 'vscode-languageserver-textdocument'
import { CompletionList, CompletionItem } from 'vscode-languageserver'
import { exec } from 'child_process'

import { existsSync, readFileSync } from 'fs'

import { URI } from 'vscode-uri'
import { findTopLevelParent, gatherFiles } from '../file/fileUtils'
import { getWordAtPosition, trimLine } from './ast'
import { debounce } from 'lodash'
import semver from 'semver'
import { bamlConfig } from '../bamlConfig'
type Notify = (
  params:
    | { type: 'error' | 'warn' | 'info'; message: string }
    // the string is a uri
    | { type: 'diagnostic'; errors: [string, Diagnostic[]][]; projects: Project[] }
    | { type: 'runtime_updated'; root_path: string; files: Record<string, string> },
) => void

const uriToRootPath = (uri: URI): string => {
  // Find the "baml_src" directory in the path
  if (uri.scheme !== 'file') {
    throw new Error(`Unsupported scheme: ${uri.scheme}`)
  }
  const found = findTopLevelParent(uri.fsPath)
  if (!found) {
    throw new Error(`No baml_src directory found in path: ${uri.fsPath}`)
  }
  return found
}

export enum GeneratorDisabledReason {
  EmptyGenerators,
  VersionMismatch,
  UserDisabled,
}

export enum GeneratorType {
  CLI = 'CLI',
  VSCode = 'VSCode',
  VSCodeCLI = 'VSCodeCLI',
}

export enum VersionCheckMode {
  Strict = 'Strict',
  None = 'None',
}

export type GeneratorStatus = {
  isReady: boolean
  generatorType: GeneratorType
  disabledReason?: GeneratorDisabledReason
  problematicGenerator?: string
  diagnostics: Map<string, Diagnostic[]>
}

class Project {
  private last_successful_runtime?: BamlWasm.WasmRuntime
  private current_runtime?: BamlWasm.WasmRuntime

  constructor(
    private wasmProject: BamlWasm.WasmProject,
    private onSuccess: (e: WasmDiagnosticError, files: Record<string, string>) => void,
  ) {}

  private checkVersion(generator: BamlWasm.WasmGeneratorConfig, isDiagnostic: boolean) {
    const message = WasmRuntime.check_version(
      generator.version,
      this.getVSCodeGeneratorVersion(),
      bamlConfig?.config?.cliPath ? GeneratorType.VSCodeCLI : GeneratorType.VSCode,
      VersionCheckMode.Strict,
      generator.output_type,
      isDiagnostic,
    )
    return message
  }

  checkVersionOnSave() {
    let firstErrorMessage: undefined | string = undefined
    this.list_generators().forEach((g) => {
      const message = this.checkVersion(g, false)
      if (message) {
        if (!firstErrorMessage) {
          firstErrorMessage = message
        }
        console.error(message)
      }
    })
    return firstErrorMessage
  }

  getGeneratorDiagnostics() {
    try {
      if (!this.current_runtime) {
        return
      }
      const generators = this.list_generators() // requires runtime
      const diagnostics = new Map<string, Diagnostic[]>()
      generators.forEach((g) => {
        const message = this.checkVersion(g, true)
        if (message) {
          const diagnostic: Diagnostic = {
            range: {
              start: { line: g.span.start_line, character: 0 },
              end: { line: g.span.end_line, character: 0 },
            },
            message: message,
            severity: DiagnosticSeverity.Error,
            source: 'baml',
          }
          diagnostics.set(URI.file(g.span.file_path).toString(), [diagnostic])
        }
      })
      return diagnostics
    } catch (e) {
      console.error(`Error getting generator diagnostics: ${e}`)
      return new Map<string, Diagnostic[]>()
    }
  }

  private getVSCodeGeneratorVersion() {
    if (bamlConfig.config?.cliPath) {
      if (bamlConfig.cliVersion) {
        return bamlConfig.cliVersion
      } else {
        throw new Error('CLI version not available')
      }
    }
    return BamlWasm.version()
  }

  update_runtime() {
    if (this.current_runtime == undefined) {
      try {
        this.current_runtime = this.wasmProject.runtime({})

        const files = this.wasmProject.files()
        const fileMap = Object.fromEntries(
          files
            .map((f): [string, string] => f.split('BAML_PATH_SPLTTER', 2) as [string, string])
            .map(([path, content]) => [URI.file(path).toString(), content]),
        )
        this.onSuccess(this.wasmProject.diagnostics(this.current_runtime), fileMap)
      } catch (e) {
        console.error(`Error updating runtime: ${e}`)
        this.current_runtime = undefined
        throw e
      }
    }
  }

  requestDiagnostics() {
    if (this.current_runtime) {
      const files = this.wasmProject.files()
      const fileMap = Object.fromEntries(
        files
          .map((f): [string, string] => f.split('BAML_PATH_SPLTTER', 2) as [string, string])
          .map(([path, content]) => [URI.file(path).toString(), content]),
      )
      const diagnostics = this.wasmProject.diagnostics(this.current_runtime)
      this.onSuccess(this.wasmProject.diagnostics(this.current_runtime), fileMap)
    }
  }

  runtime(): BamlWasm.WasmRuntime {
    const rt = this.current_runtime ?? this.last_successful_runtime
    if (!rt) {
      throw new Error(`BAML Generate failed - Project has errors.`)
    }

    return rt
  }

  files(): Record<string, string> {
    const files = this.wasmProject.files()
    return Object.fromEntries(
      files
        .map((f): [string, string] => f.split('BAML_PATH_SPLTTER', 2) as [string, string])
        .map(([path, content]) => [URI.file(path).toString(), content]),
    )
  }

  replace_all_files(project: BamlWasm.WasmProject) {
    this.wasmProject = project
    this.last_successful_runtime = this.current_runtime
    this.current_runtime = undefined
  }

  update_unsaved_file(file_path: string, content: string) {
    this.wasmProject.set_unsaved_file(file_path, content)
    if (this.current_runtime) {
      this.last_successful_runtime = this.current_runtime
    }
    this.current_runtime = undefined
  }

  save_file(file_path: string, content: string) {
    this.wasmProject.save_file(file_path, content)
    if (this.current_runtime) {
      this.last_successful_runtime = this.current_runtime
    }
    this.current_runtime = undefined
  }

  get_file(file_path: string) {
    // Read the file content
    const fileContent = readFileSync(file_path, 'utf8')

    const doc = TextDocument.create(file_path, 'plaintext', 1, fileContent)

    return doc
  }

  upsert_file(file_path: string, content: string | undefined) {
    this.wasmProject.update_file(file_path, content)
    if (this.current_runtime) {
      this.last_successful_runtime = this.current_runtime
    }
    this.current_runtime?.free()
    this.current_runtime = undefined
  }

  handleDefinitionRequest(doc: TextDocument, position: Position): LocationLink[] {
    const word = getWordAtPosition(doc, position)

    //clean non-alphanumeric characters besides underscores and periods
    const cleaned_word = trimLine(word)
    if (cleaned_word === '') {
      return []
    }

    // Search for the symbol in the runtime
    const match = this.runtime().search_for_symbol(cleaned_word)

    // If we found a match, return the location
    if (match) {
      return [
        {
          targetUri: URI.file(match.uri).toString(),
          //unused default values for now
          targetRange: {
            start: { line: 0, character: 0 },
            end: { line: 0, character: 0 },
          },
          targetSelectionRange: {
            start: { line: match.start_line, character: match.start_character },
            end: { line: match.end_line, character: match.end_character },
          },
        },
      ]
    }

    return []
  }

  handleHoverRequest(doc: TextDocument, position: Position): Hover {
    const word = getWordAtPosition(doc, position)
    const cleaned_word = trimLine(word)
    if (cleaned_word === '') {
      return { contents: [] }
    }

    const match = this.runtime().search_for_symbol(cleaned_word)

    //need to get the content of the range specified by match's start and end lines and characters
    if (match) {
      const hoverCont: { language: string; value: string }[] = []

      const range = {
        start: { line: match.start_line, character: match.start_character },
        end: { line: match.end_line, character: match.end_character },
      }

      const hoverDoc = this.get_file(URI.file(match.uri).toString())

      if (hoverDoc) {
        const hoverText = hoverDoc.getText(range)

        hoverCont.push({ language: 'baml', value: hoverText })

        return { contents: hoverCont }
      }
    }

    return { contents: [] }
  }

  list_functions(): BamlWasm.WasmFunction[] {
    let runtime = this.runtime()
    if (!runtime) {
      throw new Error(`BAML Generate failed. Project has errors.`)
    }
    return runtime.list_functions()
  }
  list_testcases(): BamlWasm.WasmTestCase[] {
    let runtime = this.runtime()
    if (!runtime) {
      throw new Error(`BAML Generate failed. Project has errors.`)
    }
    return runtime.list_testcases()
  }

  list_generators(): BamlWasm.WasmGeneratorConfig[] {
    if (this.current_runtime == undefined) {
      throw new Error(`BAML Generate failed. Project has errors.`)
    }
    return this.current_runtime.list_generators()
  }

  rootPath(): string {
    return this.wasmProject.root_dir_name
  }

  verifyCompletionRequest(doc: TextDocument, position: Position): boolean {
    const text = doc.getText()
    const offset = doc.offsetAt(position)

    let openBracesCount = 0
    let closeBracesCount = 0

    for (let i = 0; i < offset; i++) {
      if (text[i] === '{' && text[i + 1] === '{') {
        openBracesCount++
        i++ // Skip the next character
      } else if (text[i] === '}' && text[i + 1] === '}') {
        closeBracesCount++
        i++ // Skip the next character
      }
    }
    if (openBracesCount > closeBracesCount) {
      //need logic to convert line and column to index value
      let cursorIdx = 0
      const fileContent = doc.getText()

      const lines = fileContent.split('\n')
      let charCount = 0

      for (let i = 0; i < position.line; i++) {
        charCount += lines[i].length + 1 // +1 for the newline character
      }

      charCount += position.character
      cursorIdx = charCount

      const funcOfPrompt = this.runtime().check_if_in_prompt(position.line)
      if (funcOfPrompt) {
        return true
      }
    }
    return false
  }

  // Not currently debounced - lodash debounce doesn't work for this, p-debounce doesn't support trailing edge
  runGeneratorsWithoutDebounce = async ({
    onSuccess,
    onError,
  }: { onSuccess: (message: string) => void; onError: (message: string) => void }) => {
    const startMillis = performance.now()
    try {
      await Promise.all(
        this.wasmProject.run_generators().map(async (g) => {
          // Creating the tmpdir next to the output dir can cause some weird issues with vscode, if we recover
          // from an error and delete the tmpdir - vscode's explorer UI will still show baml_client.tmp even
          // though it doesn't exist anymore, and vscode has no good way of letting the user purge it from the UI
          const tmpDir = path.join(path.dirname(g.output_dir), path.basename(g.output_dir) + '.tmp')
          const backupDir = path.join(path.dirname(g.output_dir), path.basename(g.output_dir) + '.bak')

          await mkdir(tmpDir, { recursive: true })
          await Promise.all(
            g.files.map(async (f) => {
              const fpath = path.join(tmpDir, f.path_in_output_dir)
              await mkdir(path.dirname(fpath), { recursive: true })
              await writeFile(fpath, f.contents)
            }),
          )

          if (existsSync(backupDir)) {
            await rm(backupDir, { recursive: true, force: true })
          }
          if (existsSync(g.output_dir)) {
            const contents = await readdir(g.output_dir, { withFileTypes: true })
            const contentsWithSafeToRemove = await Promise.all(
              contents.map(async (c) => {
                if (c.isDirectory()) {
                  if (['__pycache__'].includes(c.name)) {
                    return { path: c.name, safeToRemove: true }
                  }
                  return { path: c.name, safeToRemove: false }
                }

                const handle = await open(path.join(g.output_dir, c.name))
                try {
                  const { bytesRead, buffer } = await handle.read(Buffer.alloc(1024), 0, 1024, 0)
                  const firstNBytes = buffer.subarray(0, bytesRead).toString('utf8')

                  return { path: c.name, safeToRemove: firstNBytes.includes('generated by BAML') }
                } finally {
                  await handle.close()
                }
              }),
            )
            const notSafeToRemove = contentsWithSafeToRemove.filter((c) => !c.safeToRemove).map((c) => c.path)
            if (notSafeToRemove.length !== 0) {
              throw new Error(
                `Output dir ${g.output_dir} contains this file(s) not generated by BAML: ${notSafeToRemove.join(', ')}`,
              )
            }
            await rename(g.output_dir, backupDir)
          }
          await rename(tmpDir, g.output_dir)
          await rm(backupDir, { recursive: true, force: true })

          return g
        }),
      )
      const endMillis = performance.now()

      onSuccess(`BAML client generated! (took ${Math.round(endMillis - startMillis)}ms)`)
    } catch (e) {
      onError(`Failed to generate BAML client: ${e}`)
    }
  }

  //runGeneratorsWithDebounce = debounce(this.runGeneratorsWithoutDebounce, 1000)
  runGeneratorsWithDebounce = this.runGeneratorsWithoutDebounce

  // render_prompt(function_name: string, params: Record<string, any>): BamlWasm.WasmPrompt {
  //   let rt = this.runtime();
  //   let func = rt.get_function(function_name)
  //   if (!func) {
  //     throw new Error(`Function ${function_name} not found`)
  //   }

  //   return func.render_prompt(rt, this.ctx, params);
  // }
}

class BamlProjectManager {
  private static instance: BamlProjectManager | null = null

  private projects: Map<string, Project> = new Map()

  constructor(private notifier: Notify) {}

  private handleMessage(e: any) {
    if (e instanceof BamlWasm.WasmDiagnosticError) {
      const diagnostics = new Map<string, Diagnostic[]>(e.all_files.map((f) => [URI.file(f).toString(), []]))
      // console.log('diagnostic filess ' + JSON.stringify(diagnostics, null, 2))

      e.errors().forEach((err) => {
        if (err.type === 'error') {
          console.error(`${err.message}, ${err.start_line}, ${err.start_column}, ${err.end_line}, ${err.end_column}`)
        }
        diagnostics.get(URI.file(err.file_path).toString())!.push({
          range: {
            start: {
              line: err.start_line,
              character: err.start_column,
            },
            end: {
              line: err.end_line,
              character: err.end_column,
            },
          },
          message: err.message,
          severity: err.type === 'error' ? DiagnosticSeverity.Error : DiagnosticSeverity.Warning,
          source: 'baml',
        })
      })

      for (const project of this.projects.values()) {
        const genDiags = project.getGeneratorDiagnostics()
        if (genDiags) {
          genDiags.forEach((diagnosticArray, uri) => {
            if (!diagnostics.has(uri)) {
              diagnostics.set(uri, [])
            }
            diagnosticArray.forEach((diagnostic) => {
              diagnostics.get(uri)!.push(diagnostic)
            })
          })
        }
      }

      // console.log('diagnostics length: ' + Array.from(diagnostics).length)
      this.notifier({
        errors: Array.from(diagnostics),
        type: 'diagnostic',
        projects: Array.from(this.projects.values()),
      })
    } else if (e instanceof Error) {
      console.error('Error linting, got error ' + e.message)
      this.notifier({ message: e.message, type: 'error' })
    } else {
      console.error('Error linting ' + JSON.stringify(e))
      this.notifier({
        message: `${e}`,
        type: 'error',
      })
    }
  }

  private wrapSync<T>(fn: () => T): T | undefined {
    try {
      return fn()
    } catch (e) {
      this.handleMessage(e)
      return undefined
    }
  }

  private async wrapAsync<T>(fn: () => Promise<T>): Promise<T | undefined> {
    return await fn().catch((e) => {
      this.handleMessage(e)
      return undefined
    })
  }

  static version(): string {
    return BamlWasm.version()
  }

  private get_project(root_path: string) {
    const project = this.projects.get(root_path)
    if (!project) {
      throw new Error(`Project not found for path: ${root_path}`)
    }

    return project
  }

  private add_project(root_path: string, files: { [path: string]: string }) {
    // console.debug(`Adding project: ${root_path}`)
    // console.log('add project project path ' + root_path)

    const project = BamlWasm.WasmProject.new(root_path, files)
    this.projects.set(
      root_path,
      new Project(project, (d, files) => {
        this.handleMessage(d)
        this.notifier({ type: 'runtime_updated', root_path, files })
      }),
    )
    return this.get_project(root_path)!
  }

  private remove_project(root_path: string) {
    this.projects.delete(root_path)
  }

  async upsert_file(path: URI, content: string | undefined) {
    // console.debug(
    //   `Upserting file: ${path}. Current projects ${this.projects.size} ${JSON.stringify(this.projects, null, 2)}`,
    // )
    await this.wrapAsync(async () => {
      console.debug(
        `Upserting file: ${path}. current projects  ${this.projects.size}  ${JSON.stringify(this.projects, null, 2)}`,
      )
      const rootPath = uriToRootPath(path)
      if (this.projects.has(rootPath)) {
        console.log('upserting ' + path.fsPath)
        const project = this.get_project(rootPath)
        project.upsert_file(path.fsPath, content)
        project.update_runtime()
      } else {
        await this.reload_project_files(path)
      }
    })
  }

  async save_file(path: URI, content: string) {
    console.debug(`Saving file: ${path}`)
    await this.wrapAsync(async () => {
      const rootPath = uriToRootPath(path)
      if (this.projects.has(rootPath)) {
        const project = this.get_project(rootPath)
        project.save_file(path.fsPath, content)
        project.update_runtime()
      } else {
        await this.reload_project_files(path)
      }
    })
  }

  update_unsaved_file(path: URI, content: string) {
    console.debug(`Updating unsaved file: ${path}`)
    this.wrapSync(() => {
      const rootPath = uriToRootPath(path)
      const project = this.get_project(rootPath)
      project.update_unsaved_file(path.fsPath, content)
      project.update_runtime()
    })
  }

  get_projects() {
    return this.projects
  }

  async touch_project(path: URI) {
    await this.wrapAsync(async () => {
      const rootPath = uriToRootPath(path)
      if (!this.projects.has(rootPath)) {
        await this.reload_project_files(path)
      }
    })
  }

  async requestDiagnostics() {
    console.debug('Requesting diagnostics')
    await this.wrapAsync(async () => {
      for (const project of this.projects.values()) {
        project.requestDiagnostics()
        this.notifier({ type: 'runtime_updated', root_path: project.rootPath(), files: project.files() })
      }
    })
  }

  // Reload all files in a project
  // Takes in a URI to any file in the project
  async reload_project_files(path: URI) {
    // console.debug(`Reloading project files: ${path}`)
    await this.wrapAsync(async () => {
      const rootPath = uriToRootPath(path)

      const files = await Promise.all(
        gatherFiles(rootPath).map(async (uri): Promise<[string, string]> => {
          const path = uri.fsPath
          const content = await readFile(path, 'utf8')
          return [path, content]
        }),
      )

      if (files.length === 0) {
        this.notifier({
          type: 'warn',
          message: `Empty baml_src directory found: ${rootPath}. See Output panel -> BAML Language Server for more details.`,
        })
      }
      console.debug(`projects ${this.projects.size}: ${JSON.stringify(this.projects, null, 2)},`)

      if (!this.projects.has(rootPath)) {
        const project = this.add_project(rootPath, Object.fromEntries(files))
        project.update_runtime()
      } else {
        const project = this.get_project(rootPath)

        project.replace_all_files(BamlWasm.WasmProject.new(rootPath, Object.fromEntries(files)))
        project.update_runtime()
      }
    })
  }

  getProjectById(id: URI): Project {
    return this.get_project(uriToRootPath(id))
  }
}

export default BamlProjectManager
