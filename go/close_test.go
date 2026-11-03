package inorbit

import (
	"net/http"
	"testing"
)

// Close closes the transport the client built and nothing it was given.
func TestCloseClosesOnlyTheTransportTheClientBuilt(t *testing.T) {
	t.Setenv("INORBIT_TOKEN", "tok")
	built, err := Load(t.Context(), WithBaseURL("https://api.test"))
	if err != nil {
		t.Fatal(err)
	}
	if built.owned == nil {
		t.Fatal("a client that built its transport does not own it")
	}
	if err := built.Close(); err != nil {
		t.Fatal(err)
	}

	given := &http.Client{Transport: &http.Transport{}}
	for name, opt := range map[string]Option{
		"WithHTTPClient": WithHTTPClient(given),
		"WithTransport":  WithTransport(&http.Transport{}),
	} {
		c, err := NewClient(WithToken("tok"), WithBaseURL("https://api.test"), opt)
		if err != nil {
			t.Fatal(name, err)
		}
		if c.owned != nil {
			t.Errorf("%s: the client owns a transport it was given", name)
		}
		if err := c.Close(); err != nil {
			t.Error(name, err)
		}
	}
	plain, err := NewClient(WithToken("tok"), WithBaseURL("https://api.test"))
	if err != nil {
		t.Fatal(err)
	}
	if plain.owned == http.DefaultTransport {
		t.Error("the client owns the shared http.DefaultTransport")
	}
	if err := plain.Close(); err != nil {
		t.Fatal(err)
	}
}
