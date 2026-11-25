import { expect, test } from "bun:test"
import { calculatorButtons } from "./faceoff-native"

test("Calculator evidence accepts real button names", () => {
  expect(calculatorButtons(["Eight", "Four", "Seven", "Multiply by", "Two", "Nine", "Three", "Equals"])).toBe(true)
})

test("the correct result alone cannot pass the Calculator interaction check", () => {
  expect(calculatorButtons(["248,171", "Close Calculator"])).toBe(false)
  expect(calculatorButtons(["Eight", "Four", "Seven", "Two", "Nine", "Three", "Equals"])).toBe(false)
})
