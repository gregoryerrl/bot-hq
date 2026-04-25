package protocol

import "time"

type AgentType string

const (
	AgentCoder   AgentType = "coder"
	AgentVoice   AgentType = "voice"
	AgentBrian   AgentType = "brian"
	AgentDiscord AgentType = "discord"
	AgentQA      AgentType = "qa"
	AgentGemma   AgentType = "gemma"
)

func (a AgentType) Valid() bool {
	switch a {
	case AgentCoder, AgentVoice, AgentBrian, AgentDiscord, AgentQA, AgentGemma:
		return true
	}
	return false
}

type AgentStatus string

const (
	StatusOnline  AgentStatus = "online"
	StatusWorking AgentStatus = "working"
	StatusOffline AgentStatus = "offline"
)

func (a AgentStatus) Valid() bool {
	switch a {
	case StatusOnline, StatusWorking, StatusOffline:
		return true
	}
	return false
}

type MessageType string

const (
	MsgHandshake MessageType = "handshake"
	MsgQuestion  MessageType = "question"
	MsgResponse  MessageType = "response"
	MsgCommand   MessageType = "command"
	MsgUpdate    MessageType = "update"
	MsgResult    MessageType = "result"
	MsgError     MessageType = "error"
	MsgFlag      MessageType = "flag"
)

func (m MessageType) Valid() bool {
	switch m {
	case MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError, MsgFlag:
		return true
	}
	return false
}

type SessionMode string

const (
	ModeBrainstorm SessionMode = "brainstorm"
	ModeImplement  SessionMode = "implement"
	ModeChat       SessionMode = "chat"
)

func (s SessionMode) Valid() bool {
	switch s {
	case ModeBrainstorm, ModeImplement, ModeChat:
		return true
	}
	return false
}

type SessionStatus string

const (
	SessionActive SessionStatus = "active"
	SessionPaused SessionStatus = "paused"
	SessionDone   SessionStatus = "done"
)

func (s SessionStatus) Valid() bool {
	switch s {
	case SessionActive, SessionPaused, SessionDone:
		return true
	}
	return false
}

type Agent struct {
	ID         string      `json:"id"`
	Name       string      `json:"name"`
	Type       AgentType   `json:"type"`
	Status     AgentStatus `json:"status"`
	Project    string      `json:"project,omitempty"`
	Meta       string      `json:"meta,omitempty"`
	Registered time.Time   `json:"registered"`
	LastSeen   time.Time   `json:"last_seen"`
}

type Session struct {
	ID      string        `json:"id"`
	Mode    SessionMode   `json:"mode"`
	Purpose string        `json:"purpose"`
	Agents  []string      `json:"agents"`
	Status  SessionStatus `json:"status"`
	Created time.Time     `json:"created"`
	Updated time.Time     `json:"updated"`
}

type Message struct {
	ID        int64       `json:"id"`
	SessionID string      `json:"session_id,omitempty"`
	FromAgent string      `json:"from_agent"`
	ToAgent   string      `json:"to_agent,omitempty"`
	Type      MessageType `json:"type"`
	Content   string      `json:"content"`
	Created   time.Time   `json:"created"`
}
