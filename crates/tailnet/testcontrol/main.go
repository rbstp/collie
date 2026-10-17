// Command testcontrol runs tailscale's in-memory control server, a DERP relay
// and a STUN server on 127.0.0.1 for the integration tests. It prints the
// control URL on stdout and serves until stdin closes.
package main

import (
	"context"
	"crypto/tls"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"sync"

	"tailscale.com/derp/derpserver"
	"tailscale.com/net/stunserver"
	"tailscale.com/tailcfg"
	"tailscale.com/tstest/integration/testcontrol"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

func main() {
	authKey := flag.String("authkey", "", "auth key every node must present")
	offline := flag.Bool("offline", false, "report every peer offline, as a stale netmap would")
	sameUser := flag.Bool("same-user", false, "own every node by one user")
	linuxNode := flag.String("linux-node", "", "report this test node as Linux")
	macNode := flag.String("mac-node", "", "report this test node as macOS")
	denyPeers := flag.Bool("deny-peers", false, "deny all peer traffic after two nodes register")
	nodeUser := flag.String("node-user", "", "assign hostname=userID for an ownership fixture")
	vmDir := flag.String("vm-dir", "", "private VM test marker directory; use fixed loopback ports")
	stunOnly := flag.Bool("stun-only", false, "serve only guest-local VM STUN")
	flag.Parse()
	if *vmDir != "" {
		var err error
		*authKey, err = vmAuthKey(*vmDir)
		if err != nil {
			log.Fatal(err)
		}
	} else if *stunOnly {
		log.Fatal("-stun-only requires -vm-dir")
	}
	if *authKey == "" {
		log.Fatal("-authkey is required")
	}

	// A local STUN response prevents repeated magicsock rebinds in the test tailnet.
	stunAddress := "127.0.0.1:0"
	if *vmDir != "" {
		stunAddress = "127.0.0.1:8791"
	}
	stun := stunserver.New(context.Background())
	if err := stun.Listen(stunAddress); err != nil {
		log.Fatal(err)
	}
	go stun.Serve()
	if *stunOnly {
		fmt.Println("VM STUN ready")
		io.Copy(io.Discard, os.Stdin)
		return
	}

	derp := httptest.NewUnstartedServer(derpserver.Handler(derpserver.New(key.NewNode(), logger.Discard)))
	// DERP upgrades an HTTP/1.1 connection; HTTP/2 would break it.
	derp.Config.TLSNextProto = map[string]func(*http.Server, *tls.Conn, http.Handler){}
	derp.Config.ErrorLog = logger.StdLogger(logger.Discard)
	if *vmDir != "" {
		derp.Listener.Close()
		listener, err := net.Listen("tcp", "127.0.0.1:8790")
		if err != nil {
			log.Fatal(err)
		}
		derp.Listener = listener
	}
	derp.StartTLS()
	defer derp.Close()

	control := &testcontrol.Server{
		DERPMap: &tailcfg.DERPMap{
			Regions: map[tailcfg.DERPRegionID]*tailcfg.DERPRegion{
				1: {
					RegionID:   1,
					RegionCode: "test",
					Nodes: []*tailcfg.DERPNode{{
						Name:             "t1",
						RegionID:         1,
						HostName:         "127.0.0.1",
						IPv4:             "127.0.0.1",
						IPv6:             "none",
						STUNPort:         stun.LocalAddr().(*net.UDPAddr).Port,
						DERPPort:         derp.Listener.Addr().(*net.TCPAddr).Port,
						InsecureForTests: true,
					}},
				},
			},
		},
		RequireAuthKey:   *authKey,
		AllNodesSameUser: *sameUser,
		// Peers are reported online, as real control does for connected nodes; collie-core
		// does not dial a peer reported offline.
		AllOnline: !*offline,
		// Non-nil so a node registering with RequestTags gets those tags.
		TagOwners: map[string][]string{},
		Logf:      logger.Discard,
	}
	var userHostname string
	var userID int64
	if *nodeUser != "" {
		hostname, value, found := strings.Cut(*nodeUser, "=")
		user, err := strconv.ParseInt(value, 10, 64)
		if !found || hostname == "" || err != nil || user <= 0 {
			log.Fatal("-node-user requires hostname=positiveUserID")
		}
		userHostname, userID = hostname, user
	}
	control.HoldMapRequest = func(request *tailcfg.MapRequest) func() {
		if info := request.Hostinfo; info != nil {
			if *linuxNode != "" && info.Hostname == *linuxNode {
				info.OS = "linux"
			}
			if *macNode != "" && info.Hostname == *macNode {
				info.OS = "macOS"
			}
		}
		if userHostname != "" {
			node := control.Node(request.NodeKey)
			if node != nil && node.Hostinfo.Hostname() == userHostname && node.User != tailcfg.UserID(userID) {
				node.User = tailcfg.UserID(userID)
				control.UpdateNode(node)
			}
		}
		return nil
	}
	var denied sync.Once
	control.ModifyFirstMapResponse = func(response *tailcfg.MapResponse, request *tailcfg.MapRequest) {
		if *denyPeers && control.NumNodes() >= 2 {
			denied.Do(func() {
				for _, node := range control.AllNodes() {
					update, err := control.MapResponse(&tailcfg.MapRequest{NodeKey: node.Key})
					if err != nil || update == nil {
						log.Fatal("cannot construct deny-policy fixture")
					}
					update.PacketFilter = nil
					update.PacketFilters = map[string][]tailcfg.FilterRule{"base": {}}
					control.AddRawMapResponse(node.Key, update)
				}
			})
			response.PacketFilter = nil
			response.PacketFilters = map[string][]tailcfg.FilterRule{"base": {}}
		}
	}
	control.HTTPTestServer = httptest.NewUnstartedServer(control)
	if *vmDir != "" {
		control.HTTPTestServer.Listener.Close()
		listener, err := net.Listen("tcp", "127.0.0.1:8789")
		if err != nil {
			log.Fatal(err)
		}
		control.HTTPTestServer.Listener = listener
	}
	control.HTTPTestServer.Start()
	defer control.HTTPTestServer.Close()

	fmt.Println(control.HTTPTestServer.URL)
	io.Copy(io.Discard, os.Stdin)
}
