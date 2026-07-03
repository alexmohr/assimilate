// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ESLintUtils } from '@typescript-eslint/utils'
import ts from 'typescript'

const createRule = ESLintUtils.RuleCreator(
  () =>
    'https://github.com/alexmohr/assimilate/blob/main/AGENTS.md#enforcement-no_string_control_flow-lint',
)

/**
 * True when `type` includes the wide `string` type anywhere (directly, or as
 * a member of a union) rather than being fully narrowed to string literals.
 * Mirrors the Rust dylint lint's `ty.is_str() || ty.is_lang_item(String)`
 * check: comparing an unconstrained string against a literal is the problem,
 * not comparing an already-narrow literal union/enum against its own member.
 */
function isWideStringType(type) {
  if (type.isUnion()) {
    return type.types.some(isWideStringType)
  }
  if (type.isStringLiteral()) {
    return false
  }
  return (type.flags & ts.TypeFlags.String) !== 0
}

function isStringLiteralNode(node) {
  return node.type === 'Literal' && typeof node.value === 'string'
}

function isNonEmptyStringLiteralNode(node) {
  return isStringLiteralNode(node) && node.value !== ''
}

function isTypeofExpression(node) {
  return node.type === 'UnaryExpression' && node.operator === 'typeof'
}

/**
 * Whether `node` is a `<expr>.key` member access where `<expr>`'s type is
 * `KeyboardEvent`. `KeyboardEvent.key` is a DOM standard string identifying a
 * physical/logical key (e.g. "ArrowDown", "Escape"); it's an external API
 * contract, not app-owned domain state, the same way the Rust workspace
 * exempts `tracing::field::Field::name()`.
 */
function isKeyboardEventKeyAccess(services, checker, node) {
  if (node.type !== 'MemberExpression' || node.computed) {
    return false
  }
  if (node.property.type !== 'Identifier' || node.property.name !== 'key') {
    return false
  }
  const tsNode = services.esTreeNodeToTSNodeMap.get(node.object)
  const objectType = checker.getTypeAtLocation(tsNode)
  return objectType.symbol?.name === 'KeyboardEvent'
}

const FUNCTION_TYPES = new Set([
  'FunctionDeclaration',
  'FunctionExpression',
  'ArrowFunctionExpression',
])

/**
 * True when `type` is composed entirely of string literal types (a proper
 * narrow union/enum), as opposed to including the wide `string` type.
 */
function isNarrowStringUnionType(type) {
  if (type.isUnion()) {
    return type.types.every(isNarrowStringUnionType)
  }
  return type.isStringLiteral()
}

/**
 * Whether `node` sits inside a function that either (a) is declared with a TS
 * type-predicate return type (`function isFoo(x): x is Foo`), or (b) returns a
 * narrow string-literal union/enum. Both are TypeScript's sanctioned
 * boundary-narrowing idioms -- the direct equivalent of the Rust workspace's
 * `from`/`from_str`/`try_from`/`deserialize` exemption: the literal
 * comparisons inside them are the intended narrowing logic, not the ad-hoc
 * control flow this rule targets elsewhere.
 */
function isInsideNarrowingFunction(context, services, checker, node) {
  return context.sourceCode.getAncestors(node).some((ancestor) => {
    if (!FUNCTION_TYPES.has(ancestor.type)) {
      return false
    }
    if (ancestor.returnType?.typeAnnotation?.type === 'TSTypePredicate') {
      return true
    }
    const tsNode = services.esTreeNodeToTSNodeMap.get(ancestor)
    const signature = checker.getSignatureFromDeclaration(tsNode)
    const returnType = signature && checker.getReturnTypeOfSignature(signature)
    return returnType != null && isNarrowStringUnionType(returnType)
  })
}

export const noStringLiteralControlFlow = createRule({
  name: 'no-string-literal-control-flow',
  meta: {
    type: 'problem',
    docs: {
      description:
        'Disallow comparing or switching on a plain string-typed value against a string literal to drive control flow; narrow the value into a string-literal union/enum at the boundary instead.',
    },
    messages: {
      comparison:
        'Do not compare a plain `string`-typed value against a literal to drive control flow. Narrow the value into a string-literal union/enum at the boundary and compare that instead.',
      switchCase:
        'Do not switch on a plain `string`-typed value using string literal cases. Narrow the value into a string-literal union/enum at the boundary and switch on that instead.',
    },
    schema: [],
  },
  defaultOptions: [],
  create(context) {
    const services = ESLintUtils.getParserServices(context)
    const checker = services.program.getTypeChecker()

    function typeOfNodeIsWideString(node) {
      const tsNode = services.esTreeNodeToTSNodeMap.get(node)
      const type = checker.getTypeAtLocation(tsNode)
      return isWideStringType(type)
    }

    return {
      BinaryExpression(node) {
        if (!['==', '===', '!=', '!=='].includes(node.operator)) {
          return
        }

        const leftIsLiteral = isNonEmptyStringLiteralNode(node.left)
        const rightIsLiteral = isNonEmptyStringLiteralNode(node.right)
        if (leftIsLiteral === rightIsLiteral) {
          // Neither or both sides are non-empty string literals: nothing to narrow
          // against (empty-string comparisons are presence checks, not domain state).
          return
        }
        const otherSide = leftIsLiteral ? node.right : node.left
        if (
          isTypeofExpression(otherSide) ||
          isKeyboardEventKeyAccess(services, checker, otherSide) ||
          isInsideNarrowingFunction(context, services, checker, node)
        ) {
          return
        }

        if (typeOfNodeIsWideString(otherSide)) {
          context.report({ node, messageId: 'comparison' })
        }
      },
      SwitchStatement(node) {
        if (
          isTypeofExpression(node.discriminant) ||
          isInsideNarrowingFunction(context, services, checker, node)
        ) {
          return
        }
        const hasStringLiteralCase = node.cases.some((c) => c.test && isStringLiteralNode(c.test))
        if (!hasStringLiteralCase) {
          return
        }

        if (typeOfNodeIsWideString(node.discriminant)) {
          context.report({ node: node.discriminant, messageId: 'switchCase' })
        }
      },
    }
  },
})
