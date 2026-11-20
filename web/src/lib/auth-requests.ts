import { z } from "zod";
import { passwordError } from "./password";

const text = (bytes: number) =>
  z.string().refine((value) => new TextEncoder().encode(value).length <= bytes);
const id = text(200).min(1);
const currentPassword = z.string().refine((value) => !passwordError(value, 1));
const newPassword = z.string().refine((value) => !passwordError(value));
const username = z.string().min(1).max(128);
const page = {
  after: text(128).optional(),
  limit: z.number().int().min(1).max(100).optional(),
};
const sensitive = { current_password: currentPassword };
const expiry = z.string().min(1).nullable().optional();

/** Browser auth has one closed shape for each current operation. */
export const authRequestSchema = z.discriminatedUnion("action", [
  z.strictObject({ action: z.literal("me") }),
  z.strictObject({
    action: z.literal("change_password"),
    ...sensitive,
    new_password: newPassword,
  }),
  z.strictObject({
    action: z.literal("create_user"),
    ...sensitive,
    username,
    password: newPassword,
    display_name: text(256).optional(),
    is_admin: z.boolean().optional(),
  }),
  z.strictObject({
    action: z.literal("list_users"),
    ...page,
    status: z.enum(["", "active", "disabled", "deleted"]).optional(),
  }),
  z.strictObject({
    action: z.literal("update_user"),
    ...sensitive,
    user_id: id,
    username: username.optional(),
    display_name: text(256).optional(),
    status: z.enum(["active", "disabled", "deleted"]).optional(),
    is_admin: z.boolean().optional(),
  }),
  z.strictObject({
    action: z.literal("reset_password"),
    ...sensitive,
    user_id: id,
    new_password: newPassword,
  }),
  z.strictObject({
    action: z.literal("create_api_key"),
    ...sensitive,
    name: text(256).min(1),
    user_id: id.optional(),
    expires_at: expiry,
  }),
  z.strictObject({
    action: z.literal("list_api_keys"),
    ...page,
    user_id: id.optional(),
  }),
  z.strictObject({
    action: z.literal("update_api_key"),
    ...sensitive,
    key_id: id,
    name: text(256).min(1).optional(),
    expires_at: expiry,
  }),
  z.strictObject({
    action: z.literal("revoke_api_key"),
    ...sensitive,
    key_id: id,
  }),
  z.strictObject({
    action: z.literal("delete_api_key"),
    ...sensitive,
    key_id: id,
  }),
]);
