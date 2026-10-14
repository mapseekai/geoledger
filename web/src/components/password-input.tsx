"use client";
import type { ComponentProps } from "react";
import { passwordError } from "@/lib/password";
import { Input } from "./ui/input";

export function PasswordInput({
  minimum = 1,
  ...props
}: ComponentProps<typeof Input> & { minimum?: number }) {
  return (
    <Input
      {...props}
      data-password-min={minimum}
      onInput={(event) => {
        event.currentTarget.setCustomValidity(
          passwordError(event.currentTarget.value, minimum),
        );
      }}
    />
  );
}
