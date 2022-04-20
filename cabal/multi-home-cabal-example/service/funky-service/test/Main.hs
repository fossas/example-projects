module Main
  ( main
  ) where

import Awesome.Prelude

import Hedgehog (success)
import Hedgehog.Main (defaultMain)

main :: IO ()
main = defaultMain [ success ]
