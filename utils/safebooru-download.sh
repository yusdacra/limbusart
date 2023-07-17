#!/usr/bin/env bash

touch all.json
for i in $(seq 0 38); do
  curl "https://safebooru.org/index.php?page=dapi&s=post&q=index&tags=project_moon&limit=100&pid=$i&json=1" > temp.json
  jq -sr '. | add' temp.json all.json > all.json.temp
  mv all.json.temp all.json
  rm temp.json
done