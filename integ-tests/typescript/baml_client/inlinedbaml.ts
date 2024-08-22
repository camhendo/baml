/*************************************************************************************************

Welcome to Baml! To use this generated code, please run one of the following:

$ npm install @boundaryml/baml
$ yarn add @boundaryml/baml
$ pnpm add @boundaryml/baml

*************************************************************************************************/

// This file was generated by BAML: do not edit it. Instead, edit the BAML
// files and re-generate this code.
//
// tslint:disable
// @ts-nocheck
// biome-ignore format: autogenerated code
/* eslint-disable */
const fileMap = {
  
  "clients.baml": "\n\n\nclient<llm> GPT4o {\n  provider openai\n  options {\n    model gpt-4o\n    api_key env.OPENAI_API_KEY\n  }\n} \n",
  "hi.baml": "\n\n// This is a BAML config file, which extends the Jinja2 templating language to write LLM functions.\n\n// class Resume {\n//   name string\n//   education Education[] @description(\"Extract in the same order listed\")\n// }\n\nclass Education {\n  school string | null @description(#\"\n    111\n  \"#)\n  degree string @description(#\"\n    2222222\n  \"#)\n}\n\nfunction ExtractResume(resume_text: string) -> Education {\n  // see clients.baml\n  client GPT4o\n\n  // The prompt uses Jinja syntax. Change the models or this text and watch the prompt preview change!\n  prompt #\"\n    Parse the following resume and return a structured representation of the data in the schema below.\n\n    Resume:\n    ---\n    {{ resume_text }}\n    ---\n\n    {# special macro to print the output instructions. #}\n    {{ ctx.output_format }}\n\n    JSON:\n  \"#\n}\n\ntest Test1 {\n  functions [ExtractResume]\n  args {\n    resume_text #\"\n      John Doe\n\n      Education\n      - University of California, Berkeley\n        - B.S. in Computer Science\n        - 2020\n\n      Skills\n      - Python\n      - Java\n      - C++\n    \"#\n  }\n}",
  "main.baml": "generator lang_python {\n  output_type python/pydantic\n  output_dir \"../python\"\n  version \"0.53.1\"\n}\n\ngenerator lang_typescript {\n  output_type typescript\n  output_dir \"../typescript\"\n  version \"0.53.1\"\n}\n\ngenerator lang_ruby {\n  output_type ruby/sorbet\n  output_dir \"../ruby\"\n  version \"0.53.1\"\n}\n",
}
export const getBamlFiles = () => {
    return fileMap;
}