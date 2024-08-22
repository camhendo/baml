###############################################################################
#
#  Welcome to Baml! To use this generated code, please run the following:
#
#  $ bundle add baml sorbet-runtime
#
###############################################################################

# This file was generated by BAML: please do not edit it. Instead, edit the
# BAML files and re-generate this code.
#
# frozen_string_literal: true
# rubocop: disable
# formatter:off
# typed: false
require "baml"
require "sorbet-runtime"

require_relative "inlined"
require_relative "partial-types"
require_relative "types"
require_relative "type-registry"

module Baml
  @instance = nil

  def self.Client
    if @instance.nil?
      @instance = BamlClient.new(runtime: Baml::Ffi::BamlRuntime.from_files("baml_src", Baml::Inlined::FILE_MAP, ENV))
    end
  
    @instance
  end

  class BamlClient
    extend T::Sig

    sig { returns(BamlStreamClient) }
    attr_reader :stream

    sig {params(runtime: Baml::Ffi::BamlRuntime).void}
    def initialize(runtime:)
      @runtime = runtime
      @ctx_manager = runtime.create_context_manager()
      @stream = BamlStreamClient.new(runtime: @runtime, ctx_manager: @ctx_manager)
    end

    sig {params(path: String).returns(BamlClient)}
    def self.from_directory(path)
      BamlClient.new(runtime: Baml::Ffi::BamlRuntime.from_directory(path, ENV))
    end

    sig {
      params(
        varargs: T.untyped,
        resume_text: String,
        baml_options: T::Hash[Symbol, T.any(Baml::TypeBuilder, Baml::ClientRegistry)]
      ).returns(Baml::Types::Education)
    }
    def ExtractResume(
        *varargs,
        resume_text:,
        baml_options: {}
    )
      if varargs.any?
        
        raise ArgumentError.new("ExtractResume may only be called with keyword arguments")
      end
      if (baml_options.keys - [:client_registry, :tb]).any?
        raise ArgumentError.new("Received unknown keys in baml_options (valid keys: :client_registry, :tb): #{baml_options.keys - [:client_registry, :tb]}")
      end

      raw = @runtime.call_function(
        "ExtractResume",
        {
          resume_text: resume_text,
        },
        @ctx_manager,
        baml_options[:tb]&.instance_variable_get(:@registry),
        baml_options[:client_registry],
      )
      (raw.parsed_using_types(Baml::Types))
    end

    

  end

  class BamlStreamClient
    extend T::Sig

    sig {params(runtime: Baml::Ffi::BamlRuntime, ctx_manager: Baml::Ffi::RuntimeContextManager).void}
    def initialize(runtime:, ctx_manager:)
      @runtime = runtime
      @ctx_manager = ctx_manager
    end

    sig {
      params(
        varargs: T.untyped,
        resume_text: String,
        baml_options: T::Hash[Symbol, T.any(Baml::TypeBuilder, Baml::ClientRegistry)]
      ).returns(Baml::BamlStream[Baml::Types::Education])
    }
    def ExtractResume(
        *varargs,
        resume_text:,
        baml_options: {}
    )
      if varargs.any?
        
        raise ArgumentError.new("ExtractResume may only be called with keyword arguments")
      end
      if (baml_options.keys - [:client_registry, :tb]).any?
        raise ArgumentError.new("Received unknown keys in baml_options (valid keys: :client_registry, :tb): #{baml_options.keys - [:client_registry, :tb]}")
      end

      raw = @runtime.stream_function(
        "ExtractResume",
        {
          resume_text: resume_text,
        },
        @ctx_manager,
        baml_options[:tb]&.instance_variable_get(:@registry),
        baml_options[:client_registry],
      )
      Baml::BamlStream[Baml::PartialTypes::Education, Baml::Types::Education].new(
        ffi_stream: raw,
        ctx_manager: @ctx_manager
      )
    end

    
  end
end