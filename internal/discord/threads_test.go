package discord

import (
	"errors"
	"testing"

	"github.com/bwmarrin/discordgo"
)

// mockThreadAPI satisfies both ThreadCreator and ThreadArchiver for tests.
type mockThreadAPI struct {
	startedThreads  []discordgo.ThreadStart
	archivedThreads []string
	startResult     *discordgo.Channel
	startErr        error
	editErr         error
}

func (m *mockThreadAPI) ThreadStartComplex(channelID string, data *discordgo.ThreadStart, _ ...discordgo.RequestOption) (*discordgo.Channel, error) {
	if m.startErr != nil {
		return nil, m.startErr
	}
	m.startedThreads = append(m.startedThreads, *data)
	if m.startResult != nil {
		return m.startResult, nil
	}
	return &discordgo.Channel{ID: "thread-" + data.Name, Name: data.Name}, nil
}

func (m *mockThreadAPI) ChannelEditComplex(channelID string, data *discordgo.ChannelEdit, _ ...discordgo.RequestOption) (*discordgo.Channel, error) {
	if m.editErr != nil {
		return nil, m.editErr
	}
	m.archivedThreads = append(m.archivedThreads, channelID)
	return &discordgo.Channel{ID: channelID}, nil
}

func TestCreateSessionThread_HappyPath(t *testing.T) {
	mock := &mockThreadAPI{}
	id, err := createSessionThread(mock, "channel-abc", "z-3-test")
	if err != nil {
		t.Fatalf("create: %v", err)
	}
	if id != "thread-z-3-test" {
		t.Errorf("id=%q want thread-z-3-test", id)
	}
	if len(mock.startedThreads) != 1 {
		t.Fatalf("expected 1 thread start; got %d", len(mock.startedThreads))
	}
	got := mock.startedThreads[0]
	if got.Name != "z-3-test" {
		t.Errorf("thread name=%q want z-3-test", got.Name)
	}
	if got.AutoArchiveDuration != 1440 {
		t.Errorf("auto_archive=%d want 1440 (24h)", got.AutoArchiveDuration)
	}
	if got.Type != discordgo.ChannelTypeGuildPublicThread {
		t.Errorf("thread type=%v want public-thread", got.Type)
	}
}

func TestCreateSessionThread_RejectsEmptyArgs(t *testing.T) {
	mock := &mockThreadAPI{}
	if _, err := createSessionThread(mock, "", "scope"); err == nil {
		t.Error("expected error on empty channel-id")
	}
	if _, err := createSessionThread(mock, "ch", ""); err == nil {
		t.Error("expected error on empty slug")
	}
}

func TestCreateSessionThread_PropagatesAPIError(t *testing.T) {
	mock := &mockThreadAPI{startErr: errors.New("discord 500")}
	if _, err := createSessionThread(mock, "ch", "scope"); err == nil {
		t.Error("expected error from API failure")
	}
}

func TestArchiveSessionThread_HappyPath(t *testing.T) {
	mock := &mockThreadAPI{}
	if err := archiveSessionThread(mock, "thread-abc"); err != nil {
		t.Fatalf("archive: %v", err)
	}
	if len(mock.archivedThreads) != 1 || mock.archivedThreads[0] != "thread-abc" {
		t.Errorf("expected archive of thread-abc; got %v", mock.archivedThreads)
	}
}

func TestArchiveSessionThread_RejectsEmpty(t *testing.T) {
	mock := &mockThreadAPI{}
	if err := archiveSessionThread(mock, ""); err == nil {
		t.Error("expected error on empty thread-id")
	}
}

func TestArchiveSessionThread_PropagatesAPIError(t *testing.T) {
	mock := &mockThreadAPI{editErr: errors.New("discord 403")}
	if err := archiveSessionThread(mock, "thread-x"); err == nil {
		t.Error("expected error from API failure")
	}
}
