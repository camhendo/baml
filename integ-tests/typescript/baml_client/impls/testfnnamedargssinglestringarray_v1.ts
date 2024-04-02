// This file is auto-generated. Do not edit this file manually.
//
// Disable formatting for this file to avoid linting errors.
// tslint:disable
// @ts-nocheck
/* eslint-disable */


import { GPT35 } from '../client';
import { TestFnNamedArgsSingleStringArray } from '../function';
import { schema } from '../json_schema';
import { LLMResponseStream } from '@boundaryml/baml-core';
import { Deserializer } from '@boundaryml/baml-core/deserializer/deserializer';


const prompt_template = `\
Return this value back to me: {//BAML_CLIENT_REPLACE_ME_MAGIC_input.myStringArray//}\
`;

const deserializer = new Deserializer<string>(schema, {
  $ref: '#/definitions/TestFnNamedArgsSingleStringArray_output'
});

const v1 = async (
  args: {
    myStringArray: string[]
  }
): Promise<string> => {
  const myStringArray = args.myStringArray.map(x => x);
  
  const result = await GPT35.run_prompt_template(
    prompt_template,
    [
      "{//BAML_CLIENT_REPLACE_ME_MAGIC_input.myStringArray//}",
    ],
    {
      "{//BAML_CLIENT_REPLACE_ME_MAGIC_input.myStringArray//}": myStringArray,
    }
  );

  return deserializer.coerce(result.generated);
};

const v1_stream = (
  args: {
    myStringArray: string[]
  }
): LLMResponseStream<string> => {
  const myStringArray = args.myStringArray.map(x => x);
  
  const stream = GPT35.run_prompt_template_stream(
    prompt_template,
    [
      "{//BAML_CLIENT_REPLACE_ME_MAGIC_input.myStringArray//}",
    ],
    {
      "{//BAML_CLIENT_REPLACE_ME_MAGIC_input.myStringArray//}": myStringArray,
    }
  );

  return new LLMResponseStream<string>(
    stream,
    (partial: string) => deserializer.coerce(partial),
    (final: string) => deserializer.coerce(final),
  );
};

TestFnNamedArgsSingleStringArray.registerImpl('v1', v1, v1_stream);


