package inorbit_test

import (
	"context"
	"errors"
	"fmt"
	"log"
	"log/slog"
	"os"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
	"github.com/inorbithr/sdk/go/public/models"
)

// The examples call the real API, so `go test` compiles them without running them.

func ExampleNewClient() {
	// An API token from the console or `iohr token create`.
	c, err := inorbit.NewClient(inorbit.WithToken(os.Getenv("INORBIT_TOKEN")))
	if err != nil {
		log.Fatal(err)
	}
	me, err := public.New(c).Me(context.Background())
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(me.Value.Subject)
}

func ExampleNewClient_key() {
	// An API key, exchanged for short-lived tokens that the client caches and refreshes.
	c, err := inorbit.NewClient(
		inorbit.WithKey(os.Getenv("INORBIT_KEY_ID"), os.Getenv("INORBIT_KEY_SECRET")),
		inorbit.WithScopes("identity:read", "radar:read"),
		inorbit.WithTimeout(10*time.Second),
	)
	if err != nil {
		log.Fatal(err)
	}
	digests, err := public.New(c).Radar().ListDigests(context.Background(), nil)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(len(digests.Value.Digests))
}

func ExampleFromEnv() {
	// "" reads INORBIT_TOKEN, or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES; a
	// profile name such as "ACME_CI" reads INORBIT_ACME_CI_* and nothing else.
	c, err := inorbit.FromEnv("")
	if err != nil {
		log.Fatal(err)
	}
	_ = public.New(c)
}

// vaultTokens reads tokens your platform already issues.
type vaultTokens struct{}

func (vaultTokens) Token(context.Context) (inorbit.Token, error) {
	return inorbit.Token{Access: os.Getenv("TOKEN_FROM_VAULT"), ExpiresAt: time.Now().Add(time.Hour)}, nil
}

func ExampleWithTokenProvider() {
	c, err := inorbit.NewClient(inorbit.WithTokenProvider(vaultTokens{}))
	if err != nil {
		log.Fatal(err)
	}
	_ = public.New(c)
}

func ExampleAPIError() {
	c, err := inorbit.FromEnv("")
	if err != nil {
		log.Fatal(err)
	}
	_, err = public.New(c).Events().GetEndpoint(context.Background(), "ep_123")
	var api *inorbit.APIError
	switch {
	case errors.As(err, &api) && api.Code == inorbit.CodeNotFound:
		fmt.Println("no such endpoint")
	case errors.As(err, &api) && api.Code == inorbit.CodeForbidden:
		fmt.Println("this credential's scopes do not allow it")
	case err != nil:
		log.Fatal(err)
	}
}

func ExampleSlogHook() {
	// Debug records per answer, Warn when a call fails; never a token, path, query or body.
	logger := slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelDebug}))
	c, err := inorbit.FromEnv("", inorbit.WithHook(inorbit.SlogHook(logger)))
	if err != nil {
		log.Fatal(err)
	}
	_ = public.New(c)
}

func Example_pagination() {
	c, err := inorbit.FromEnv("")
	if err != nil {
		log.Fatal(err)
	}
	// All<Operation> follows the page tokens; break whenever you have enough.
	for delivery, err := range public.New(c).Events().AllListDeliveries(context.Background(), "ep_123", nil) {
		if err != nil {
			log.Fatal(err)
		}
		fmt.Println(delivery.ID)
	}
}

func ExampleStream() {
	// A key with events:read. Each event arrives as it happens; an error ends the
	// stream, and breaking out of the loop or ending ctx closes it. WithStreams
	// (StreamsSocket) carries every stream of the client over one /v1/ws socket instead.
	c, err := inorbit.FromEnv("", inorbit.WithStreams(inorbit.StreamsSSE))
	if err != nil {
		log.Fatal(err)
	}
	types := "key.created"
	ctx, cancel := context.WithTimeout(context.Background(), time.Hour)
	defer cancel()
	for ev, err := range public.New(c).Events().StreamEvents(ctx, &models.EventsStreamEventsParams{Types: &types}) {
		if err != nil {
			log.Fatal(err)
		}
		fmt.Println(ev.Type, ev.ID)
	}
}
