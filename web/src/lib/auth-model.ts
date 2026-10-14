export type User = {
  id: string;
  username: string;
  display_name: string;
  status: "active" | "disabled" | "deleted";
  is_admin: boolean;
  must_change_password: boolean;
  created_at: string;
};
export type ApiKey = {
  id: string;
  name: string;
  key_prefix: string;
  user_id: string;
  created_at: string;
  expires_at: string | null;
  revoked_at: string | null;
  last_used_at: string | null;
};
export type AuthResult = { session: string; user: User };
