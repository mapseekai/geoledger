package geoledger

import (
	"context"
	"encoding/json"
	pb "github.com/mapseekai/geoledger/sdk/go/internal/geoledgerv1"
	"google.golang.org/protobuf/proto"
)

func (c *Client) invoke(ctx context.Context, operation string, input []byte) (proto.Message, error) {
	switch operation {
	case "Info":
		var request pb.Empty
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Info(ctx, &request)
	case "CreateProject":
		var request pb.NameRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.CreateProject(ctx, &request)
	case "ListProjects":
		var request pb.PageRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.ListProjects(ctx, &request)
	case "GetProject":
		var request pb.ProjectRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.GetProject(ctx, &request)
	case "SetMember":
		var request pb.MemberRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.SetMember(ctx, &request)
	case "CreateDataset":
		var request pb.DatasetRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.CreateDataset(ctx, &request)
	case "ListDatasets":
		var request pb.ProjectPageRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.ListDatasets(ctx, &request)
	case "CreateWorkspace":
		var request pb.ProjectRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.CreateWorkspace(ctx, &request)
	case "ListWorkspaces":
		var request pb.ProjectPageRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.ListWorkspaces(ctx, &request)
	case "GetWorkspace":
		var request pb.WorkspaceRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.GetWorkspace(ctx, &request)
	case "Save":
		var request pb.SaveRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Save(ctx, &request)
	case "Discard":
		var request pb.VersionRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Discard(ctx, &request)
	case "Features":
		var request pb.FeaturesRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Features(ctx, &request)
	case "Diff":
		var request pb.DiffRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Diff(ctx, &request)
	case "Conflicts":
		var request pb.DiffRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Conflicts(ctx, &request)
	case "History":
		var request pb.HistoryRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.History(ctx, &request)
	case "Commit":
		var request pb.CommitRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Commit(ctx, &request)
	case "Audit":
		var request pb.HistoryRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Audit(ctx, &request)
	case "Publish":
		var request pb.PublishRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Publish(ctx, &request)
	case "Resolve":
		var request pb.ResolveRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Resolve(ctx, &request)
	case "Rebase":
		var request pb.ResolveRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Rebase(ctx, &request)
	case "Restore":
		var request pb.RestoreRequest
		if err := json.Unmarshal(input, &request); err != nil {
			return nil, invalid("invalid request")
		}
		return c.rpc.Restore(ctx, &request)
	default:
		return nil, invalid("unknown operation")
	}
}
