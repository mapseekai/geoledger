namespace rs geoledger.v1

// Commands and result JSON follow the same v1 contract as HTTP/gRPC.
// authorization contains "Bearer <GL_API_TOKEN>" when enabled.
exception ApiError {
  1: required string code,
  2: required string message
}

struct Empty {

}

struct JsonReply {
  1: required string json
}

struct ExecuteRequest {
  1: required string command_json
}

struct StatusRequest {
  1: required i32 limit
}

struct CommitRequest {
  1: required string message,
  2: required string author
}

struct ImportRequest {
  1: required string dataset,
  2: required string schema,
  3: required string table,
  4: required string author,
  5: required string message
}

struct LogRequest {
  1: required string reference,
  2: required i32 limit
}

struct DiffRequest {
  1: required string from,
  2: required string to,
  3: required i32 limit
}

struct BranchRequest {
  1: required string name,
  2: required string from
}

struct ReferenceRequest {
  1: required string reference
}

struct MergeRequest {
  1: required string reference,
  2: required string author,
  3: required string message
}

struct ResetRequest {
  1: required string target,
  2: required bool hard
}

struct RestoreRequest {
  1: required bool discard
}

service GeoLedger {
  JsonReply execute(1: ExecuteRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply status(1: StatusRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply import(1: ImportRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply commit(1: CommitRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply log(1: LogRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply diff(1: DiffRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply branch(1: BranchRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply switch(1: ReferenceRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply merge(1: MergeRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply revert(1: MergeRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply reset(1: ResetRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply restore(1: RestoreRequest request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply continue(1: Empty request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply abort(1: Empty request, 2: optional string authorization) throws (1: ApiError error),
  JsonReply recover(1: Empty request, 2: optional string authorization) throws (1: ApiError error)
}
