package main

import (
	"fmt"

	jsoniter "github.com/json-iterator/go"
	"github.com/prometheus/client_golang/prometheus"
)

func main() {
	prometheus.DefMaxAge.Hours()
	jsoniter.Valid([]byte(""))
	fmt.Println("Hello world!")
}
